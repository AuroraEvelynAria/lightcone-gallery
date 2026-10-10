//! Schema 定义与版本化迁移。
//!
//! 迁移策略：读取 `PRAGMA user_version`，按版本号顺序执行增量脚本。
//! 新增变更时**追加**一个新的 `if version < N` 分支，不要修改历史分支。

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{AppError, AppResult};

/// 当前 schema 版本，与下面最后一个迁移分支保持一致。
pub const SCHEMA_VERSION: i32 = 9;

/// v1：图库、图片、相册、标签、缩略图缓存与键值设置。
const MIGRATION_V1: &str = r#"
-- 用户添加的根目录
CREATE TABLE libraries (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    path            TEXT    NOT NULL UNIQUE,
    name            TEXT    NOT NULL,
    created_at      INTEGER NOT NULL,
    last_scanned_at INTEGER
);

-- 索引到的图片文件
CREATE TABLE images (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    library_id  INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    path        TEXT    NOT NULL UNIQUE,
    filename    TEXT    NOT NULL,
    extension   TEXT    NOT NULL,
    width       INTEGER,
    height      INTEGER,
    file_size   INTEGER NOT NULL,
    modified_at INTEGER NOT NULL,
    -- 预留：EXIF DateTimeOriginal；当前为 NULL，排序回退到 modified_at
    taken_at    INTEGER,
    is_favorite INTEGER NOT NULL DEFAULT 0,
    rating      INTEGER NOT NULL DEFAULT 0,
    indexed_at  INTEGER NOT NULL
);

CREATE INDEX idx_images_library    ON images(library_id);
CREATE INDEX idx_images_filename   ON images(filename);
CREATE INDEX idx_images_modified   ON images(modified_at DESC);
CREATE INDEX idx_images_favorite   ON images(is_favorite) WHERE is_favorite = 1;

-- 相册（用户手动组织）
CREATE TABLE albums (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    name           TEXT    NOT NULL UNIQUE,
    description    TEXT,
    cover_image_id INTEGER REFERENCES images(id) ON DELETE SET NULL,
    created_at     INTEGER NOT NULL
);

CREATE TABLE album_images (
    album_id INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
    image_id INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    position INTEGER NOT NULL DEFAULT 0,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (album_id, image_id)
);

CREATE INDEX idx_album_images_image ON album_images(image_id);

-- 标签（可与相册并存，用于交叉筛选）
CREATE TABLE tags (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT    NOT NULL UNIQUE,
    color      TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE image_tags (
    image_id INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    tag_id   INTEGER NOT NULL REFERENCES tags(id)   ON DELETE CASCADE,
    PRIMARY KEY (image_id, tag_id)
);

CREATE INDEX idx_image_tags_tag ON image_tags(tag_id);

-- 缩略图磁盘缓存登记表（文件本体在 app data 目录下）
CREATE TABLE thumbnails (
    image_id     INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    size         INTEGER NOT NULL,
    path         TEXT    NOT NULL,
    width        INTEGER NOT NULL,
    height       INTEGER NOT NULL,
    generated_at INTEGER NOT NULL,
    PRIMARY KEY (image_id, size)
);

-- 键值设置
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

/// v2（阶段一）：Pixiv 作品维度的索引。
///
/// 与 `images` 的分工：`images` 是通用图库的索引（标签 / 相册 / 缩略图都挂在它上面），
/// `artworks` 只收录文件名符合 `{pid}_p{num}` 的文件，是作品维度的索引。
const MIGRATION_V2: &str = r#"
CREATE TABLE artworks (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    pid        TEXT    NOT NULL,
    file_path  TEXT    NOT NULL,
    -- 文件名里的 _p{num}。这一列不能省：`123456_p0.jpg` 与 `123456_p1.jpg` 的 pid 相同，
    -- 若只对 pid 建唯一索引，多页作品的第 2 页起会被静默丢弃。
    page       INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

-- 去重：同一作品的同一页只保留一条
CREATE UNIQUE INDEX idx_artworks_pid_page ON artworks(pid, page);
CREATE INDEX idx_artworks_pid ON artworks(pid);
"#;

/// v3（阶段二）：缩略图路径入库 + 命名对齐 + 扫描根目录落盘。
///
/// 三处改动各自的理由：
/// 1. `file_path` 改名 `original_path`。查询命令对外返回的是
///    `pid / original_path / thumbnail_path`，列名跟着走，免得每层都要翻译一次。
/// 2. 新增 `thumbnail_path`。缩略图在扫描阶段就生成好，前端拿列表直接就能渲染，
///    不必每张卡片再问一次后端要路径。
/// 3. `artwork_roots` 记录扫描过的根目录。原图要经 `asset://` 显示，而该协议的作用域
///    **只存在于内存中**（见 lib.rs 的启动流程），不落盘就没法在重启后重新授权。
const MIGRATION_V3: &str = r#"
ALTER TABLE artworks RENAME COLUMN file_path TO original_path;
ALTER TABLE artworks ADD COLUMN thumbnail_path TEXT;

CREATE TABLE artwork_roots (
    path       TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL
);
"#;

/// 4. `ignored_artworks` 记录「被移出作品库」的 `(pid, page)`。
///
/// 没有这张表时「移除」只是删行，重新扫描同一目录会把它们再收回来 —— 从用户
/// 视角看就是「移除了又回来了」。有了它，移除才是真的移除。
/// 刻意按 `(pid, page)` 而不是 `id` 记：重扫会拿到新的自增 id，pid 才是稳定的身份。
///
/// `IF NOT EXISTS` 不是随手加的：这一版上线前踩过「表建好了、`user_version` 没落盘」
/// 的半吊子状态（见 `apply` 的注释），已经处于该状态的库要靠它自愈。
const MIGRATION_V4: &str = r#"
CREATE TABLE IF NOT EXISTS ignored_artworks (
    pid        TEXT NOT NULL,
    page       INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (pid, page)
);
"#;

/// 5. 画师维度：`artists` 表 + `artworks` / `images` 的 `artist_uid` 列。
///
/// pixez 这类下载器落盘时就按 `<画师名>_<画师ID>/` 分好了目录，所以画师信息
/// **不需要联网**，一直躺在路径里（实测本地 98.8% 的行能解析出来）。
/// 解析规则与「为什么 uid 才是身份」见 `crate::artists` 的模块注释。
///
/// 刻意**不加外键**：画师行的来源是路径，路径才是真相，`artists` 只是把它物化成
/// 可查询、可建索引的形状。加外键会把「派生值」说成「权威实体」，而且会让
/// 扫描时的插入顺序变成硬约束（必须先有 artists 行）。同理，删掉一个画师的所有
/// 作品后 artists 行会留下 —— 这是有意的，它同时充当「见过这位画师」的记忆。
const MIGRATION_V5: &str = r#"
CREATE TABLE IF NOT EXISTS artists (
    uid        TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

ALTER TABLE artworks ADD COLUMN artist_uid TEXT;
ALTER TABLE images   ADD COLUMN artist_uid TEXT;

CREATE INDEX IF NOT EXISTS idx_artworks_artist ON artworks(artist_uid);
CREATE INDEX IF NOT EXISTS idx_images_artist   ON images(artist_uid);
"#;

/// 6. 联网取回的元数据：`illust_meta`（每 PID 一行）+ `illust_tags`（标签多对多）。
///
/// 标签**只能**从接口来（路径里没有），所以这张表是「按标签归类」的唯一来源。
/// 画师也在里面存一份（`artist_uid` / `artist_name`）：路径派生的 uid 实测与接口
/// 10/10 一致，但**名字会被 Windows 文件名规则清洗**（接口的 `-273℃/望歩` 在目录里
/// 是 `-273℃望歩`），所以显示名以接口为准。
///
/// 失败的抓取也占一行（`error` 有值、其余为空），而不是丢掉重来：
/// 记下来才知道「这个 PID 试过了、失败了」，否则每次都会从头再撞一遍。
/// 想重试就按 `error IS NOT NULL` 挑出来。
const MIGRATION_V6: &str = r#"
CREATE TABLE IF NOT EXISTS illust_meta (
    pid            TEXT PRIMARY KEY,
    title          TEXT NOT NULL DEFAULT '',
    artist_uid     TEXT,
    artist_name    TEXT,
    artist_account TEXT,
    page_count     INTEGER NOT NULL DEFAULT 0,
    x_restrict     INTEGER NOT NULL DEFAULT 0,
    sanity_level   INTEGER NOT NULL DEFAULT 0,
    illust_type    INTEGER NOT NULL DEFAULT 0,
    create_date    TEXT,
    fetched_at     INTEGER NOT NULL,
    error          TEXT
);

CREATE TABLE IF NOT EXISTS illust_tags (
    pid TEXT NOT NULL,
    tag TEXT NOT NULL,
    PRIMARY KEY (pid, tag)
);

CREATE INDEX IF NOT EXISTS idx_illust_tags_tag ON illust_tags(tag);
CREATE INDEX IF NOT EXISTS idx_illust_meta_artist ON illust_meta(artist_uid);
"#;

/// 7. `images` 补上 `pid` 列。
///
/// 通用图库一直缺这一列，导致它和 Pixiv 维度之间没有桥：弹窗显示不了 Pixiv 标签、
/// 图库页也没法按标签筛选（`illust_tags` 是按 `pid` 挂的）。
/// 与 `artist_uid` 一样是**从路径派生**的，不联网 —— 文件名就是 `{pid}_p{n}.{ext}`。
const MIGRATION_V7: &str = r#"
ALTER TABLE images ADD COLUMN pid TEXT;

CREATE INDEX IF NOT EXISTS idx_images_pid ON images(pid);
"#;

/// 8. `illust_meta` 补一列 `missing`，把「作品在 Pixiv 上没了」标成终态。
///
/// 之前 `Missing` 和可重试的失败共用 `error` 列，导致批量抓取的挑选条件
/// （「没有 `error IS NULL` 的行」）会**每次都把已删除的作品重新挑出来**：
/// 既对着永远不会回来的作品反复发请求，又让「待补全」永远归不了零。
///
/// 分成两态：
/// - `missing = 1` —— Pixiv 明确返回 404，作品被删 / 转私密 / 下架。终态，不重试。
/// - `error IS NOT NULL AND missing = 0` —— 网络、超时、限流。可重试。
///
/// 留 `reset_missing_meta` 这个出口：转私密的作品以后可能放回来，
/// 「终态」不该是单向门。
const MIGRATION_V8: &str = r#"
ALTER TABLE illust_meta ADD COLUMN missing INTEGER NOT NULL DEFAULT 0;

-- 全新库走到这里时一并把订正做掉；已经跑过 v8 的库由 v9 补上（见 MIGRATION_V9）。
UPDATE illust_meta SET missing = 1 WHERE error = '作品不存在或已删除';

CREATE INDEX IF NOT EXISTS idx_illust_meta_missing ON illust_meta(missing);
"#;

/// 9. 数据订正：把 v8 之前写下的「已删除」记录补上 `missing = 1`。
///
/// **为什么不合并进 v8**：v8 的开发过程中被 `tauri dev` 自动重启跑过一次，
/// 那条记录已经落盘；之后再往 v8 里加语句也不会重跑 —— 迁移一旦应用就不再回头。
/// 这正是「新增变更要追加新分支、不要改历史分支」这条规矩的现实版本。
///
/// 判据是文案而不是「所有 error 非空」：v8 之前的 `mark_missing` 是当时唯一会写
/// 这行的路径，文案固定。可重试的网络失败文案各不相同，不能一并标成终态。
const MIGRATION_V9: &str = r#"
UPDATE illust_meta SET missing = 1 WHERE error = '作品不存在或已删除';
"#;

pub fn migrate(conn: &Connection) -> AppResult<()> {
    let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    if version < 1 {
        apply(conn, 1, MIGRATION_V1)?;
    }
    if version < 2 {
        apply(conn, 2, MIGRATION_V2)?;
    }
    if version < 3 {
        apply(conn, 3, MIGRATION_V3)?;
    }
    if version < 4 {
        apply(conn, 4, MIGRATION_V4)?;
    }
    if version < 5 {
        apply(conn, 5, MIGRATION_V5)?;
    }
    if version < 6 {
        apply(conn, 6, MIGRATION_V6)?;
    }
    if version < 7 {
        apply(conn, 7, MIGRATION_V7)?;
    }
    if version < 8 {
        apply(conn, 8, MIGRATION_V8)?;
    }
    if version < 9 {
        apply(conn, 9, MIGRATION_V9)?;
    }

    // 后续迁移在此追加： if version < 10 { apply(conn, 10, MIGRATION_V10)?; }

    // v3 之前入库的作品行从没登记过扫描根目录。这里反推补一次，同时覆盖两条路径：
    // 「从老库升级上来」以及「库已经停在 v3、但 v3 迁移早于这段逻辑上线」。
    // 登记过就只花一次 COUNT 查询，直接跳过。
    let registered: i64 = conn.query_row("SELECT COUNT(*) FROM artwork_roots", [], |row| row.get(0))?;
    if registered == 0 {
        backfill_artwork_roots(conn)?;
    }

    // v5 之前入库的行没有 artist_uid。这个回填只该跑一次，而且**不能**用
    // 「artists 表是否为空」来判断：用户的图库若全是非 pixez 命名的目录，
    // 回填完 artists 依然是空的，那样每次启动都会把整张表扫一遍。
    // 所以用一个哨兵键记下来。
    let backfilled: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [ARTISTS_BACKFILLED_KEY],
            |row| row.get(0),
        )
        .optional()?;
    if backfilled.is_none() {
        backfill_artists(conn)?;
    }

    let pid_backfilled: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [PIDS_BACKFILLED_KEY],
            |row| row.get(0),
        )
        .optional()?;
    if pid_backfilled.is_none() {
        backfill_image_pids(conn)?;
    }

    // 收尾自检：跑完全部迁移后，库里的版本号必须正好等于 SCHEMA_VERSION。
    // 少了这道检查，「加了迁移却忘了改常量」或者「某一步没把版本写下去」都会静默通过，
    // 然后在下一次启动时以 `table ... already exists` 的形式炸出来 —— 真的踩过。
    let final_version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if final_version != SCHEMA_VERSION {
        return Err(AppError::InvalidInput(format!(
            "迁移结束后库版本是 {final_version}，期望 {SCHEMA_VERSION}：有迁移没同步 SCHEMA_VERSION"
        )));
    }

    Ok(())
}

/// 回填哨兵键。值只用来表示「跑过了」，内容不重要。
const ARTISTS_BACKFILLED_KEY: &str = "artists_backfilled";
const PIDS_BACKFILLED_KEY: &str = "image_pids_backfilled";

/// 从 `images.filename` 解析 `{pid}_p{n}` 里的 pid 并写回。
///
/// 与画师回填同样的哨兵策略：只跑一次。解析用的是 `artworks::parse_pid` ——
/// 和作品库同一套规则，所以两边的 pid 一定对得上，`illust_tags` 的 join 才成立。
fn backfill_image_pids(conn: &Connection) -> AppResult<()> {
    let now = crate::db::now_unix();
    let tx = conn.unchecked_transaction()?;
    {
        let mut update = tx.prepare("UPDATE images SET pid = ?1 WHERE id = ?2")?;
        let rows: Vec<(i64, String)> = {
            let mut stmt = tx.prepare("SELECT id, filename FROM images")?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, filename) in rows {
            if let Some((pid, _)) = crate::artworks::parse_pid(&filename) {
                update.execute(rusqlite::params![pid, id])?;
            }
        }
    }
    tx.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![PIDS_BACKFILLED_KEY, now.to_string()],
    )?;
    tx.commit()?;
    Ok(())
}

/// 遍历两张表现有的路径，解析画师、登记进 `artists`、把 uid 写回行上。
///
/// 幂等（`artist_uid` 会被重算成同样的值），但只在升级那一次跑 —— 见 `migrate`
/// 里对哨兵的说明。两张表都过一遍：通用图库和作品库是两套索引，同一个文件可能
/// 只出现在其中一边。
fn backfill_artists(conn: &Connection) -> AppResult<()> {
    let now = crate::db::now_unix();
    // `unchecked_transaction` 是给 `&Connection` 用的（普通的 `transaction` 要 `&mut`）。
    // 中途任何一步失败，`tx` 析构时会自动回滚，哨兵也就不会写下去。
    let tx = conn.unchecked_transaction()?;
    {
        let mut update = tx.prepare("UPDATE artworks SET artist_uid = ?1 WHERE id = ?2")?;
        let rows: Vec<(i64, String)> = {
            let mut stmt = tx.prepare("SELECT id, original_path FROM artworks")?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, path) in rows {
            let artist = crate::artists::parse_artist(&path);
            crate::artists::remember_artist(&tx, &artist, now)?;
            if let Some((uid, _)) = artist {
                update.execute(rusqlite::params![uid, id])?;
            }
        }
    }
    {
        let mut update = tx.prepare("UPDATE images SET artist_uid = ?1 WHERE id = ?2")?;
        let rows: Vec<(i64, String)> = {
            let mut stmt = tx.prepare("SELECT id, path FROM images")?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, path) in rows {
            let artist = crate::artists::parse_artist(&path);
            crate::artists::remember_artist(&tx, &artist, now)?;
            if let Some((uid, _)) = artist {
                update.execute(rusqlite::params![uid, id])?;
            }
        }
    }
    tx.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![ARTISTS_BACKFILLED_KEY, now.to_string()],
    )?;
    tx.commit()?;
    Ok(())
}

/// 跑一版迁移，并把它自己的版本号一起提交。
///
/// DDL 与 `user_version` 必须在同一个事务里。分开写的后果不是理论上的：开发时
/// 改到一半被重启，就出现过「`CREATE TABLE` 成功、版本号没落盘」，下一次启动
/// 重跑同一版迁移直接撞 `table ... already exists`，应用起不来。
/// `user_version` 存在数据库头部，本身是受事务保护的，所以这样写是原子的。
fn apply(conn: &Connection, version: i32, sql: &str) -> AppResult<()> {
    conn.execute_batch("BEGIN")?;
    match conn.execute_batch(sql) {
        Ok(()) => {
            conn.pragma_update(None, "user_version", version)?;
            conn.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(err) => {
            // 回滚本身失败不重要，原始错误才是要报出去的
            let _ = conn.execute_batch("ROLLBACK");
            Err(err.into())
        }
    }
}

/// 从现有 `original_path` 反推作品所在的目录，补登进 `artwork_roots`。
///
/// 不做这一步的话，老库里的作品原图不在 `asset://` 的授权范围内（该作用域只存在于
/// 内存中，靠 `artwork_roots` 在每次启动时重建），点开检视弹窗会加载失败，
/// 而用户完全不知道为什么。同一目录只登记一次；库里没有作品行时直接空跑。
fn backfill_artwork_roots(conn: &Connection) -> AppResult<()> {
    let mut dirs: Vec<String> = {
        let mut stmt = conn.prepare("SELECT original_path FROM artworks")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.filter_map(Result::ok)
            .filter_map(|path| {
                Path::new(&path)
                    .parent()
                    .map(|dir| dir.to_string_lossy().into_owned())
            })
            .collect()
    };

    if dirs.is_empty() {
        return Ok(());
    }
    dirs.sort_unstable();
    dirs.dedup();

    let now = super::now_unix();
    let mut stmt = conn.prepare(
        "INSERT INTO artwork_roots (path, created_at) VALUES (?1, ?2)
         ON CONFLICT(path) DO NOTHING",
    )?;
    for dir in &dirs {
        stmt.execute(params![dir, now])?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_creates_schema_and_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();

        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        for table in [
            "libraries",
            "images",
            "albums",
            "album_images",
            "tags",
            "image_tags",
            "thumbnails",
            "settings",
            "artworks",
            "artwork_roots",
        ] {
            let found: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(found, 1, "缺少表 {table}");
        }
    }

    #[test]
    fn deleting_library_cascades_to_images() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrate(&conn).unwrap();

        conn.execute(
            "INSERT INTO libraries (id, path, name, created_at) VALUES (1, 'C:/pics', 'pics', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO images (library_id, path, filename, extension, file_size, modified_at, indexed_at)
             VALUES (1, 'C:/pics/a.jpg', 'a.jpg', 'jpg', 100, 0, 0)",
            [],
        )
        .unwrap();

        conn.execute("DELETE FROM libraries WHERE id = 1", []).unwrap();

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM images", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 0, "级联删除未生效");
    }

    /// 「表建好了、版本号没落盘」的半吊子状态必须能自愈。
    ///
    /// 这不是假想：开发时改到一半被 `tauri dev` 重启，就留下过这样一个库，
    /// 下一次启动直接撞 `table ignored_artworks already exists` 起不来。
    #[test]
    fn recovers_from_a_half_applied_migration() {
        let conn = Connection::open_in_memory().unwrap();

        // 造一个「停在 v3、但 v4 的表已经建好」的库
        conn.execute_batch(MIGRATION_V1).unwrap();
        conn.execute_batch(MIGRATION_V2).unwrap();
        conn.execute_batch(MIGRATION_V3).unwrap();
        conn.pragma_update(None, "user_version", 3).unwrap();
        conn.execute_batch(MIGRATION_V4).unwrap();

        migrate(&conn).expect("半吊子状态应当自愈，而不是报 already exists");

        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION, "自愈后版本号应当补上");
    }

    /// v5 的画师回填：老库里的行没有 `artist_uid`，升级时从路径反推补上。
    ///
    /// 这条守着「画师不用联网」这个承诺 —— 回填一旦坏掉，界面上所有画师分组都会
    /// 变成空的，而路径里明明有。
    #[test]
    fn v5_backfills_artists_from_paths() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

        // 造一个停在 v4 的库
        conn.execute_batch(MIGRATION_V1).unwrap();
        conn.execute_batch(MIGRATION_V2).unwrap();
        conn.execute_batch(MIGRATION_V3).unwrap();
        conn.execute_batch(MIGRATION_V4).unwrap();
        conn.pragma_update(None, "user_version", 4).unwrap();

        conn.execute(
            "INSERT INTO libraries (id, path, name, created_at) VALUES (1, 'C:/pics', 'pics', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO images (library_id, path, filename, extension, file_size, modified_at, indexed_at)
             VALUES (1, 'C:/pics/Ciza__41583659/1_p0.jpg', '1_p0.jpg', 'jpg', 1, 0, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO artworks (pid, original_path, page, created_at)
             VALUES ('1', 'C:/pics/QuAn__6657532/1_p0.jpg', 0, 0)",
            [],
        )
        .unwrap();
        // 直接躺在图库根目录下，解析不出画师，应当留 NULL 而不是报错
        conn.execute(
            "INSERT INTO artworks (pid, original_path, page, created_at)
             VALUES ('2', 'C:/pics/2_p0.jpg', 0, 0)",
            [],
        )
        .unwrap();

        migrate(&conn).unwrap();

        let images_uid: Option<String> = conn
            .query_row("SELECT artist_uid FROM images", [], |row| row.get(0))
            .unwrap();
        assert_eq!(images_uid.as_deref(), Some("41583659"));

        let artworks_uid: Option<String> = conn
            .query_row("SELECT artist_uid FROM artworks WHERE pid = '1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(artworks_uid.as_deref(), Some("6657532"));

        let orphan: Option<String> = conn
            .query_row("SELECT artist_uid FROM artworks WHERE pid = '2'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(orphan, None, "解析不出画师的行应当是 NULL");

        // 名字保住了末尾下划线（本地数据里有 47 个这样的画师）
        let name: String = conn
            .query_row("SELECT name FROM artists WHERE uid = '6657532'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(name, "QuAn_");

        assert_eq!(crate::artists::list(&conn).unwrap().len(), 2, "两位画师");
    }

    /// 回填只该跑一次：哨兵在，就不该再扫全表。
    ///
    /// 不能用「`artists` 表是否为空」当判据 —— 图库若全是非 pixez 命名的目录，
    /// 回填完 `artists` 依然是空的，那样每次启动都会把整张表扫一遍。
    #[test]
    fn artist_backfill_runs_only_once() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        let sentinel: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'artists_backfilled'",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        assert!(sentinel.is_some(), "首次迁移应当写下哨兵");

        // 把 artists 清空再迁移一次：哨兵在，就不该重新回填
        conn.execute("DELETE FROM artists", []).unwrap();
        migrate(&conn).unwrap();
        assert!(
            crate::artists::list(&conn).unwrap().is_empty(),
            "哨兵应当阻止重复回填"
        );
    }

    /// v1 老库一路升到当前版本，必须保留原有数据。
    #[test]
    fn upgrades_existing_v1_database_without_data_loss() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

        // 手工造一个「已经跑过 v1」的旧库
        conn.execute_batch(MIGRATION_V1).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute(
            "INSERT INTO libraries (id, path, name, created_at) VALUES (1, 'C:/pics', 'pics', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO images (library_id, path, filename, extension, file_size, modified_at, indexed_at)
             VALUES (1, 'C:/pics/a.jpg', 'a.jpg', 'jpg', 1, 0, 0)",
            [],
        )
        .unwrap();

        let before: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, 1, "前置条件：旧库版本应为 1");

        migrate(&conn).unwrap();

        let after: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(after, SCHEMA_VERSION);

        let images: i64 = conn
            .query_row("SELECT COUNT(*) FROM images", [], |row| row.get(0))
            .unwrap();
        assert_eq!(images, 1, "升级不得丢失原有图片记录");

        let libraries: i64 = conn
            .query_row("SELECT COUNT(*) FROM libraries", [], |row| row.get(0))
            .unwrap();
        assert_eq!(libraries, 1, "升级不得丢失原有图库记录");

        let artworks: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'artworks'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(artworks, 1, "v2 应新增 artworks 表");
    }

    /// v2 -> v3：老库里的作品记录必须原样保留，旧列改名要跟过去、新列要补上。
    /// 用户现网就是这条路径（v2 库里已有作品记录）。
    #[test]
    fn upgrades_existing_v2_database_and_keeps_artworks() {
        let conn = Connection::open_in_memory().unwrap();

        conn.execute_batch(MIGRATION_V1).unwrap();
        conn.execute_batch(MIGRATION_V2).unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();
        conn.execute(
            "INSERT INTO artworks (pid, file_path, page, created_at)
             VALUES ('123456', 'C:/a/123456_p0.jpg', 0, 7)",
            [],
        )
        .unwrap();

        migrate(&conn).unwrap();

        let (pid, original, thumb): (String, String, Option<String>) = conn
            .query_row(
                "SELECT pid, original_path, thumbnail_path FROM artworks",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(pid, "123456");
        assert_eq!(original, "C:/a/123456_p0.jpg", "改名不得丢数据");
        assert_eq!(thumb, None, "新列应为空，等扫描阶段回填");

        let roots: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'artwork_roots'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(roots, 1, "v3 应新增 artwork_roots 表");

        // 老库里的作品从没登记过扫描根目录，迁移必须从 original_path 反推补上，
        // 否则重启后这些原图不在 asset:// 授权范围内，弹窗会加载失败
        let backfilled: Vec<String> = {
            let mut stmt = conn.prepare("SELECT path FROM artwork_roots").unwrap();
            let rows = stmt.query_map([], |row| row.get(0)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert_eq!(backfilled, vec!["C:/a".to_string()], "应从原图路径反推出所在目录");

        // 幂等：再跑一次迁移不得重复登记
        migrate(&conn).unwrap();
        let again: i64 = conn
            .query_row("SELECT COUNT(*) FROM artwork_roots", [], |row| row.get(0))
            .unwrap();
        assert_eq!(again, 1);
    }

    /// 回归：唯一约束必须是 (pid, page) 而不是 pid，
    /// 否则多页作品从第 2 页起会被 INSERT OR IGNORE 静默丢弃。
    #[test]
    fn artworks_dedupes_by_pid_and_page_not_by_pid_alone() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        let insert = |pid: &str, page: i64, path: &str| {
            conn.execute(
                "INSERT INTO artworks (pid, original_path, page, created_at)
                 VALUES (?1, ?2, ?3, 0)
                 ON CONFLICT(pid, page) DO NOTHING",
                rusqlite::params![pid, path, page],
            )
            .unwrap()
        };

        assert_eq!(insert("123456", 0, "C:/a/123456_p0.jpg"), 1);
        assert_eq!(insert("123456", 1, "C:/a/123456_p1.jpg"), 1, "同一作品的第 2 页必须能入库");
        assert_eq!(insert("123456", 2, "C:/a/123456_p2.jpg"), 1);
        assert_eq!(insert("123456", 0, "C:/other/123456_p0.jpg"), 0, "重复的 (pid, page) 应被去重");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM artworks WHERE pid = '123456'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 3, "多页作品应保留 3 条记录，而不是塌缩成 1 条");

        let pages: Vec<i64> = {
            let mut stmt = conn
                .prepare("SELECT page FROM artworks WHERE pid = '123456' ORDER BY page")
                .unwrap();
            let rows = stmt.query_map([], |row| row.get(0)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert_eq!(pages, vec![0, 1, 2]);
    }
}

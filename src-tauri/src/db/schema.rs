//! Schema 定义与版本化迁移。
//!
//! 迁移策略：读取 `PRAGMA user_version`，按版本号顺序执行增量脚本。
//! 新增变更时**追加**一个新的 `if version < N` 分支，不要修改历史分支。

use rusqlite::Connection;

use crate::error::AppResult;

/// 当前 schema 版本，与下面最后一个迁移分支保持一致。
pub const SCHEMA_VERSION: i32 = 2;

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

pub fn migrate(conn: &Connection) -> AppResult<()> {
    let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    if version < 1 {
        conn.execute_batch(MIGRATION_V1)?;
    }
    if version < 2 {
        conn.execute_batch(MIGRATION_V2)?;
    }

    // 后续迁移在此追加： if version < 3 { conn.execute_batch(MIGRATION_V3)?; }

    if version != SCHEMA_VERSION {
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
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

    /// v1 老库升级到 v2 必须保留原有数据（用户现网就是这个路径）。
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
        assert_eq!(after, 2);
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

    /// 回归：唯一约束必须是 (pid, page) 而不是 pid，
    /// 否则多页作品从第 2 页起会被 INSERT OR IGNORE 静默丢弃。
    #[test]
    fn artworks_dedupes_by_pid_and_page_not_by_pid_alone() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        let insert = |pid: &str, page: i64, path: &str| {
            conn.execute(
                "INSERT INTO artworks (pid, file_path, page, created_at)
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

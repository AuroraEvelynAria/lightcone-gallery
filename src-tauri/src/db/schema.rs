//! Schema 定义与版本化迁移。
//!
//! 迁移策略：读取 `PRAGMA user_version`，按版本号顺序执行增量脚本。
//! 新增变更时**追加**一个新的 `if version < N` 分支，不要修改历史分支。

use rusqlite::Connection;

use crate::error::AppResult;

/// 当前 schema 版本，与下面最后一个迁移分支保持一致。
pub const SCHEMA_VERSION: i32 = 1;

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

pub fn migrate(conn: &Connection) -> AppResult<()> {
    let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    if version < 1 {
        conn.execute_batch(MIGRATION_V1)?;
    }

    // 后续迁移在此追加： if version < 2 { conn.execute_batch(MIGRATION_V2)?; }

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
}

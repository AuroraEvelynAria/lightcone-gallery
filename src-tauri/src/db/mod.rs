//! 数据库层：连接建立、schema 迁移与数据模型。
//!
//! 选用 rusqlite（同步 API）+ `Mutex<Connection>` 单连接模型：
//! 图库场景下写入集中在索引阶段，读取都是小查询，单连接配合 WAL 已经足够，
//! 且避免了连接池与 rusqlite 版本耦合。若后续需要并行读取，可换成 r2d2 连接池。

pub mod models;
pub mod schema;

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use crate::error::AppResult;

/// 打开（必要时创建）数据库并完成迁移。
pub fn open(path: &Path) -> AppResult<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous  = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;",
    )?;

    schema::migrate(&conn)?;
    Ok(conn)
}

/// 当前 Unix 时间戳（秒）。全库统一用秒级整数存时间，避免额外依赖 chrono。
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 由目录路径推导图库名称（取最后一段目录名）。
pub fn library_name_from_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.to_string())
        .unwrap_or_else(|| path.to_string())
}

/// 判断是否为唯一约束冲突，用于把 SQLite 的原始报错翻译成「名称已存在」这类提示。
pub fn is_unique_violation(err: &rusqlite::Error) -> bool {
    match err {
        rusqlite::Error::SqliteFailure(inner, message) => {
            inner.code == rusqlite::ErrorCode::ConstraintViolation
                && message
                    .as_deref()
                    .map(|text| text.contains("UNIQUE"))
                    .unwrap_or(false)
        }
        _ => false,
    }
}

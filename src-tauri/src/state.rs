//! 应用共享状态与「在阻塞线程池上访问数据库」的辅助函数。
//!
//! rusqlite 是同步 API，直接在异步命令里调用会占住 async runtime 的工作线程。
//! 所有涉及数据库 / 图像解码的操作统一走 `tauri::async_runtime::spawn_blocking`。

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::{AppError, AppResult};

pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    /// 缩略图缓存根目录（app data 下）
    pub thumbnail_dir: PathBuf,
    /// 自动补全元数据是否还在跑。
    ///
    /// 那个循环一次要跑几十分钟，必须有办法中途叫停 —— `stop_meta_fetch` 把它翻成
    /// `false`，循环每抓完一批检查一次。放在这里而不是前端：真正的循环在 Rust 这边，
    /// 前端的「停止」只是个信号，能停下来的前提是循环自己愿意看这个标志。
    pub meta_running: Arc<AtomicBool>,
}

impl AppState {
    pub fn new(conn: Connection, thumbnail_dir: PathBuf) -> Self {
        Self {
            db: Arc::new(Mutex::new(conn)),
            thumbnail_dir,
            meta_running: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// 把一段需要 `&mut Connection` 的工作搬到阻塞线程池上执行。
pub async fn with_db<T, F>(state: &AppState, task: F) -> AppResult<T>
where
    F: FnOnce(&mut Connection) -> AppResult<T> + Send + 'static,
    T: Send + 'static,
{
    let db = state.db.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = db
            .lock()
            .map_err(|_| AppError::Other("数据库连接锁已损坏（存在 panic 残留）".into()))?;
        task(&mut conn)
    })
    .await
    .map_err(|err| AppError::Other(format!("后台任务执行失败: {err}")))?
}

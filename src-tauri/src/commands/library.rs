//! 图库（根目录）相关命令：增删、扫描与进度事件。

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::db::models::{Library, ScanProgress, ScanSummary};
use crate::db::{library_name_from_path, now_unix};
use crate::error::{AppError, AppResult};
use crate::scanner;
use crate::state::{AppState, with_db};

/// 扫描进度事件名，前端用 `listen('scan://progress')` 订阅。
pub const SCAN_PROGRESS_EVENT: &str = "scan://progress";

const LIBRARY_SELECT: &str = "
    SELECT l.id,
           l.path,
           l.name,
           l.created_at,
           l.last_scanned_at,
           (SELECT COUNT(*) FROM images i WHERE i.library_id = l.id) AS image_count
    FROM libraries l";

pub fn fetch_library(conn: &Connection, id: i64) -> AppResult<Library> {
    conn.query_row(&format!("{LIBRARY_SELECT} WHERE l.id = ?1"), [id], Library::from_row)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("图库 {id} 不存在")))
}

/// 把目录加入 asset:// 协议白名单，否则 WebView 无法直接加载这些文件。
///
/// 注意：该 scope 只存在于内存中，每次进程启动都要重新授权（见 `lib.rs` 的 setup）。
pub fn grant_asset_scope(app: &AppHandle, dir: &Path) -> AppResult<()> {
    app.asset_protocol_scope()
        .allow_directory(dir, true)
        .map_err(|err| AppError::Other(format!("授权本地资源目录失败: {err}")))
}

#[tauri::command]
pub async fn list_libraries(state: State<'_, AppState>) -> AppResult<Vec<Library>> {
    with_db(&state, |conn| {
        let mut stmt = conn.prepare(&format!("{LIBRARY_SELECT} ORDER BY l.name COLLATE NOCASE"))?;
        let rows = stmt.query_map([], Library::from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await
}

#[tauri::command]
pub async fn add_library(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> AppResult<Library> {
    let dir = PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(AppError::InvalidPath(format!("目录不存在: {path}")));
    }

    let library = with_db(&state, move |conn| {
        let name = library_name_from_path(&path);
        // 同一目录重复添加（SAF 授权被 ROM 记住时秒回，很容易连点两次）：
        // 幂等返回已有的那条，而不是把 UNIQUE 约束错误原样甩到界面上
        match conn.execute(
            "INSERT INTO libraries (path, name, created_at) VALUES (?1, ?2, ?3)",
            params![path, name, now_unix()],
        ) {
            Ok(_) => fetch_library(conn, conn.last_insert_rowid()),
            Err(err) if err.to_string().contains("UNIQUE constraint failed: libraries.path") => {
                let id: i64 = conn
                    .query_row(
                        "SELECT id FROM libraries WHERE path = ?1",
                        params![path],
                        |row| row.get(0),
                    )
                    .map_err(|err| AppError::Other(format!("读取已有图库失败: {err}")))?;
                fetch_library(conn, id)
            }
            Err(err) => Err(err.into()),
        }
    })
    .await?;

    grant_asset_scope(&app, Path::new(&library.path))?;
    Ok(library)
}

#[tauri::command]
pub async fn remove_library(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    with_db(&state, move |conn| {
        let affected = conn.execute("DELETE FROM libraries WHERE id = ?1", [id])?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("图库 {id} 不存在")));
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn scan_library(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> AppResult<ScanSummary> {
    let (path, name) = with_db(&state, move |conn| {
        conn.query_row(
            "SELECT path, name FROM libraries WHERE id = ?1",
            [id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("图库 {id} 不存在")))
    })
    .await?;

    let db = state.db.clone();
    let emitter = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = db
            .lock()
            .map_err(|_| AppError::Other("数据库连接锁已损坏".into()))?;
        let mut on_progress = |progress: ScanProgress| {
            let _ = emitter.emit(SCAN_PROGRESS_EVENT, progress);
        };
        scanner::scan_library(&mut conn, id, &name, Path::new(&path), &mut on_progress)
    })
    .await
    .map_err(|err| AppError::Other(format!("扫描任务执行失败: {err}")))?
}

#[tauri::command]
pub async fn scan_all_libraries(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Vec<ScanSummary>> {
    let targets = with_db(&state, |conn| {
        let mut stmt = conn.prepare("SELECT id, path, name FROM libraries ORDER BY id")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await?;

    let db = state.db.clone();
    let emitter = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = db
            .lock()
            .map_err(|_| AppError::Other("数据库连接锁已损坏".into()))?;

        let mut summaries = Vec::with_capacity(targets.len());
        for (id, path, name) in targets {
            let mut on_progress = |progress: ScanProgress| {
                let _ = emitter.emit(SCAN_PROGRESS_EVENT, progress);
            };
            match scanner::scan_library(&mut conn, id, &name, Path::new(&path), &mut on_progress) {
                Ok(summary) => summaries.push(summary),
                // 单个图库失败（例如外接盘已拔出）不应中断其余图库
                Err(err) => eprintln!("扫描图库 {id} 失败: {err}"),
            }
        }
        Ok(summaries)
    })
    .await
    .map_err(|err| AppError::Other(format!("扫描任务执行失败: {err}")))?
}

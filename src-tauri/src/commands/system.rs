//! 全局统计与缓存维护命令。

use serde::Serialize;
use tauri::State;

use crate::db::models::Stats;
use crate::error::AppResult;
use crate::state::{AppState, with_db};
use crate::thumbnails;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheUsage {
    pub bytes: u64,
    pub files: u64,
}

#[tauri::command]
pub async fn library_stats(state: State<'_, AppState>) -> AppResult<Stats> {
    with_db(&state, |conn| {
        let scalar = |sql: &str| -> AppResult<i64> { Ok(conn.query_row(sql, [], |row| row.get(0))?) };

        Ok(Stats {
            images: scalar("SELECT COUNT(*) FROM images")?,
            libraries: scalar("SELECT COUNT(*) FROM libraries")?,
            albums: scalar("SELECT COUNT(*) FROM albums")?,
            tags: scalar("SELECT COUNT(*) FROM tags")?,
            favorites: scalar("SELECT COUNT(*) FROM images WHERE is_favorite = 1")?,
            total_bytes: scalar("SELECT COALESCE(SUM(file_size), 0) FROM images")?,
        })
    })
    .await
}

#[tauri::command]
pub async fn thumbnail_cache_usage(state: State<'_, AppState>) -> AppResult<CacheUsage> {
    let cache_dir = state.thumbnail_dir.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let (bytes, files) = thumbnails::cache_usage(&cache_dir);
        Ok(CacheUsage { bytes, files })
    })
    .await
    .map_err(|err| crate::error::AppError::Other(format!("统计缓存失败: {err}")))?
}

#[tauri::command]
pub async fn clear_thumbnail_cache(state: State<'_, AppState>) -> AppResult<()> {
    let cache_dir = state.thumbnail_dir.clone();

    tauri::async_runtime::spawn_blocking(move || thumbnails::clear_cache(&cache_dir))
        .await
        .map_err(|err| crate::error::AppError::Other(format!("清理缓存失败: {err}")))?
}

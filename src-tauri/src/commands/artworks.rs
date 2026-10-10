//! 阶段一：作品（Pixiv）维度的命令。
//! 阶段二：补上作品缩略图（有损 WebP），供前端虚拟滚动网格取图。

use std::path::PathBuf;

use rusqlite::{OptionalExtension, params};
use tauri::State;

use crate::artworks;
use crate::db::models::{Artwork, ArtworkPage, ArtworkScanSummary};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, with_db};
use crate::thumbnails;

const DEFAULT_LIMIT: i64 = 60;
const MAX_LIMIT: i64 = 200;

/// 接收一个本地文件夹路径，遍历其中的图片，解析 `{pid}_p{num}` 后写入 `artworks` 表。
///
/// 返回的 `inserted` / `duplicates` 直接反映基于唯一索引的去重效果：
/// 重复扫描同一目录时 `inserted` 为 0、`duplicates` 等于匹配到的文件数。
#[tauri::command]
pub async fn scan_artworks_folder(
    state: State<'_, AppState>,
    path: String,
) -> AppResult<ArtworkScanSummary> {
    let root = PathBuf::from(&path);
    if !root.is_dir() {
        return Err(AppError::InvalidPath(format!("目录不存在: {path}")));
    }

    with_db(&state, move |conn| artworks::scan_folder(conn, &root)).await
}

/// 分页返回作品列表：从第 `offset` 条开始，返回 `limit` 条。
#[tauri::command]
pub async fn list_artworks(
    state: State<'_, AppState>,
    offset: Option<i64>,
    limit: Option<i64>,
) -> AppResult<ArtworkPage> {
    let offset = offset.unwrap_or(0).max(0);
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

    with_db(&state, move |conn| {
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM artworks", [], |row| row.get(0))?;

        // pid 存成 TEXT，排序必须按数值比较，否则 "999" 会排在 "12345" 之后
        let mut stmt = conn.prepare(
            "SELECT id, pid, file_path, page, created_at
             FROM artworks
             ORDER BY CAST(pid AS INTEGER) DESC, page ASC, id ASC
             LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt.query_map(params![limit, offset], Artwork::from_row)?;
        let items = rows.collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(ArtworkPage {
            items,
            total,
            offset,
            limit,
        })
    })
    .await
}

/// 返回作品缩略图的绝对路径（有损 WebP）。
///
/// 与 `thumbnail_for` 同构，只是源路径取自 `artworks.file_path`，缓存也落在
/// `thumbnails/artworks/` 下，不会和通用图库的 `thumbnails/images/` 撞车。
#[tauri::command]
pub async fn thumbnail_for_artwork(
    state: State<'_, AppState>,
    id: i64,
    size: Option<u32>,
) -> AppResult<String> {
    let source: PathBuf = with_db(&state, move |conn| {
        conn.query_row("SELECT file_path FROM artworks WHERE id = ?1", [id], |row| {
            row.get::<_, String>(0)
        })
        .optional()?
        .map(PathBuf::from)
        .ok_or_else(|| AppError::NotFound(format!("作品 {id} 不存在")))
    })
    .await?;

    let size = thumbnails::normalize_size(size.unwrap_or(thumbnails::DEFAULT_SIZE));
    thumbnails::ensure_offloaded(
        state.thumbnail_dir.clone(),
        thumbnails::Owner::Artwork,
        id,
        source,
        size,
    )
    .await
}

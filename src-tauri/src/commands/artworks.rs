//! 阶段一：作品（Pixiv）维度的命令。
//! 阶段二：扫描后批量生成有损 WebP 缩略图并入库，列表直接带路径返回。

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use rusqlite::{OptionalExtension, params};
use tauri::{AppHandle, Emitter, State};

use crate::artworks;
use crate::commands::grant_asset_scope;
use crate::db::models::{Artwork, ArtworkPage, ArtworkScanSummary};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, with_db};
use crate::thumbnails;

const DEFAULT_LIMIT: i64 = 60;
const MAX_LIMIT: i64 = 200;

/// 缩略图生成进度事件的频道名，与前端 `THUMBNAIL_PROGRESS_EVENT` 对应。
const THUMBNAIL_PROGRESS_EVENT: &str = "artworks://thumbnail-progress";

/// 进度上报的最小间隔（张）。全量生成时每张都推一次会刷爆事件通道。
const PROGRESS_STEP: usize = 25;

/// 所有作品查询共用的列清单，保证 `Artwork::from_row` 能按名字取到每一列。
const ARTWORK_COLUMNS: &str = "id, pid, original_path, thumbnail_path, page, created_at";

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ThumbnailProgress {
    processed: usize,
    total: usize,
}

/// 接收一个本地文件夹路径：遍历图片、解析 `{pid}_p{num}` 入库，
/// 再为缺缩略图的作品批量生成有损 WebP 并把路径写回数据库。
///
/// 返回的 `inserted` / `duplicates` 直接反映基于唯一索引的去重效果：
/// 重复扫描同一目录时 `inserted` 为 0、`duplicates` 等于匹配到的文件数。
/// 三个 `thumbnails*` 字段反映本轮缩略图 pass 的产出——缓存仍然新鲜的一律跳过，
/// 所以重复扫描几乎不花时间。
#[tauri::command]
pub async fn scan_artworks_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> AppResult<ArtworkScanSummary> {
    let root = PathBuf::from(&path);
    if !root.is_dir() {
        return Err(AppError::InvalidPath(format!("目录不存在: {path}")));
    }

    // 原图要经 asset:// 显示（点击卡片弹窗），而该协议的作用域只存在于内存中，
    // 扫完必须立刻授权，否则用户得重启应用才能看到原图。
    grant_asset_scope(&app, &root)?;

    let cache_dir = state.thumbnail_dir.clone();
    let scanned = root.clone();
    let emitter = app.clone();
    let reported = AtomicUsize::new(0);

    with_db(&state, move |conn| {
        let mut summary = artworks::scan_folder(conn, &scanned)?;

        let on_progress = |done: usize, total: usize| {
            // 节流：只在跨过 PROGRESS_STEP 或全部完成时上报。
            // `done` 由原子自增产生、每个值唯一，所以不会有两个线程同时通过这道判断。
            if done < total && done - reported.load(Ordering::Relaxed) < PROGRESS_STEP {
                return;
            }
            reported.store(done, Ordering::Relaxed);
            let _ = emitter.emit(
                THUMBNAIL_PROGRESS_EVENT,
                ThumbnailProgress {
                    processed: done,
                    total,
                },
            );
        };

        let batch =
            artworks::generate_thumbnails(conn, &cache_dir, thumbnails::ARTWORK_SIZE, &on_progress)?;

        summary.thumbnails_generated = batch.generated;
        summary.thumbnails_skipped = batch.skipped;
        summary.thumbnails_failed = batch.failed;
        Ok(summary)
    })
    .await
}

/// 分页返回作品列表：从第 `offset` 条开始，返回 `limit` 条。
///
/// 每条都带着 `original_path` 与 `thumbnail_path`：缩略图在扫描阶段就生成好了，
/// 前端拿列表直接就能渲染，不必为每张卡片再往返一次 IPC。
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
        let mut stmt = conn.prepare(&format!(
            "SELECT {ARTWORK_COLUMNS} FROM artworks
             ORDER BY CAST(pid AS INTEGER) DESC, page ASC, id ASC
             LIMIT ?1 OFFSET ?2"
        ))?;
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

/// 把作品移出作品库：删掉数据库记录与对应的缩略图缓存文件。
///
/// **绝不碰原图** —— 这是「移出索引」，不是删除文件。原图始终留在用户自己的
/// 目录里，这也是整个项目的硬约束（应用对图库目录只读不写）。
///
/// 缩略图缓存顺手清掉：它是按 `artworks.id` 命名的，行删了以后重扫会拿到新的
/// 自增 id，旧文件永远不会再被读到，留着就是孤儿。
///
/// 返回实际删掉的行数（传进来的 id 可能有一部分已经不存在了）。
#[tauri::command]
pub async fn remove_artworks(state: State<'_, AppState>, ids: Vec<i64>) -> AppResult<usize> {
    let cache_dir = state.thumbnail_dir.clone();
    with_db(&state, move |conn| artworks::remove(conn, &cache_dir, &ids)).await
}
/// 重新生成一张作品的缩略图并把新路径写回数据库，返回该路径。
///
/// 正常路径上前端**不该**调它 —— 列表里已经带着 `thumbnail_path` 了。
/// 这是自愈回退：缓存目录被清空（侧栏的「清理」按钮）、或盘上文件被删之后，
/// 卡片上的 `<img>` 会加载失败，此时前端调一次这里把图补回来。
#[tauri::command]
pub async fn thumbnail_for_artwork(state: State<'_, AppState>, id: i64) -> AppResult<String> {
    let source: PathBuf = with_db(&state, move |conn| {
        conn.query_row(
            "SELECT original_path FROM artworks WHERE id = ?1",
            [id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(PathBuf::from)
        .ok_or_else(|| AppError::NotFound(format!("作品 {id} 不存在")))
    })
    .await?;

    let path = thumbnails::ensure_offloaded(
        state.thumbnail_dir.clone(),
        thumbnails::Owner::Artwork,
        id,
        source,
        thumbnails::ARTWORK_SIZE,
    )
    .await?;

    // 新路径写回库，下次列表就直接带对了
    let stored = path.clone();
    with_db(&state, move |conn| {
        conn.execute(
            "UPDATE artworks SET thumbnail_path = ?1 WHERE id = ?2",
            params![stored, id],
        )?;
        Ok(())
    })
    .await?;

    Ok(path)
}

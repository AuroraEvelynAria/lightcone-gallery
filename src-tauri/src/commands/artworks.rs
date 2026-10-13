//! 阶段一：作品（Pixiv）维度的命令。
//! 阶段二：扫描后批量生成有损 WebP 缩略图并入库，列表直接带路径返回。

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use rusqlite::{OptionalExtension, params};
use tauri::{AppHandle, Emitter, State};

use crate::artworks::{self, ARTWORK_ORDER};
use crate::commands::grant_asset_scope;
use crate::db::models::{
    ArtistSummary, Artwork, ArtworkAspect, ArtworkPage, ArtworkScanSummary, RATING_ALL_AGES,
    RATING_ANY, RATING_NSFW,
};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, with_db};
use crate::thumbnails;

const DEFAULT_LIMIT: i64 = 60;
const MAX_LIMIT: i64 = 200;

/// 缩略图生成进度事件的频道名，与前端 `THUMBNAIL_PROGRESS_EVENT` 对应。
const THUMBNAIL_PROGRESS_EVENT: &str = "artworks://thumbnail-progress";

/// 进度上报的最小间隔（张）。全量生成时每张都推一次会刷爆事件通道。
const PROGRESS_STEP: usize = 25;

/// 所有作品查询共用的列清单，保证 `artwork_from_row` 能按名字取到每一列。
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
    artist_uids: Option<Vec<String>>,
    pixiv_tags: Option<Vec<String>>,
    rating: Option<i64>,
) -> AppResult<ArtworkPage> {
    let offset = offset.unwrap_or(0).max(0);
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let artist_uids = clean_filter(artist_uids);
    let pixiv_tags = clean_filter(pixiv_tags);
    let rating = rating.unwrap_or(RATING_ANY);

    with_db(&state, move |conn| {
        let (filter, binds) = artwork_filter(&artist_uids, &pixiv_tags, rating);

        let total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM artworks WHERE 1 = 1{filter}"),
            rusqlite::params_from_iter(binds.iter()),
            |row| row.get(0),
        )?;

        // pid 存成 TEXT，排序必须按数值比较，否则 "999" 会排在 "12345" 之后。
        // 这句与 `list_aspects` 共用常量 —— 瀑布流靠两边顺序一致才能对齐。
        let mut stmt = conn.prepare(&format!(
            "SELECT {ARTWORK_COLUMNS} FROM artworks WHERE 1 = 1{filter} {ARTWORK_ORDER}
             LIMIT ? OFFSET ?"
        ))?;
        let mut all = binds;
        all.push(rusqlite::types::Value::Integer(limit));
        all.push(rusqlite::types::Value::Integer(offset));
        let rows = stmt.query_map(rusqlite::params_from_iter(all.iter()), artwork_from_row)?;
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

/// 读出一行作品，顺手把**指向旧尺寸目录**的缩略图路径判成缺失。
///
/// 缩略图缓存按 `<owner>/<size>/` 分层，所以改动 `ARTWORK_SIZE` 之后库里存的还是
/// 老路径 —— 文件在、图能开，只是分辨率不够（不报错，只是糊）。返回 `None` 之后
/// 前端会走一次重生成回退（`thumbnail_for_artwork`），那边用的是当前尺寸，
/// 补完还会写回库，于是主人滑到哪张补哪张，不必重扫整个库。
///
/// 判据只有「路径的父目录名是不是当前尺寸」，不碰文件系统：`list_artworks` 是分页
/// 热路径，一页几十条各来一次 `stat` 不值得。
fn artwork_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Artwork> {
    let mut artwork = Artwork::from_row(row)?;
    if !thumbnails::is_current_size(artwork.thumbnail_path.as_deref(), thumbnails::ARTWORK_SIZE) {
        artwork.thumbnail_path = None;
    }
    Ok(artwork)
}

/// 清掉空白项并去重，避免前端传来 `["", " "]` 这种把 SQL 拼出个空 IN 列表。
fn clean_filter(values: Option<Vec<String>>) -> Vec<String> {
    let mut cleaned: Vec<String> = values
        .unwrap_or_default()
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    cleaned.sort();
    cleaned.dedup();
    cleaned
}

/// 生成 `?, ?, ?` —— 个数由调用方给，值本身仍然走绑定，不拼进 SQL。
fn placeholders(count: usize) -> String {
    std::iter::repeat("?").take(count).collect::<Vec<_>>().join(", ")
}

/// 分级的 SQL 片段。`RATING_ANY` 返回 `None`（不加条件），其余两种情况各给一句。
///
/// 分级取自 `illust_meta.x_restrict`，所以要子查询 —— `pid` 是那张表的主键，
/// 走的是索引，不必为它再建列。
///
/// **没抓过元数据的作品两个分级都不属于**：它们是「未知」，不是「全年龄」。
/// 硬把未知算进全年龄会让主人在「全年龄」图库里看到不该看的东西，所以宁可漏，
/// 界面上另有一行提示这类作品的条数。
fn rating_clause(rating: i64) -> Option<&'static str> {
    match rating {
        RATING_ANY => None,
        RATING_ALL_AGES => Some(" AND pid IN (SELECT pid FROM illust_meta WHERE x_restrict = 0)"),
        RATING_NSFW => Some(" AND pid IN (SELECT pid FROM illust_meta WHERE x_restrict >= 1)"),
        // 未知取值一律当「不限」处理：前端传错不该把列表清空成 0 条
        _ => None,
    }
}

/// 作品库筛选条件的统一拼装：画师（任一）+ 标签（同时包含）+ 分级。
///
/// `list_artworks` 与标签图库的计数**共用这一份** —— 两边各写一遍必然漂移，
/// 界面上「侧栏说 537 条、点进去却是另一个数」就是这么来的。
///
/// 值全部走绑定，只有占位符是拼出来的（与 `list_images` 同一套做法）。
fn artwork_filter(
    artist_uids: &[String],
    pixiv_tags: &[String],
    rating: i64,
) -> (String, Vec<rusqlite::types::Value>) {
    let mut filter = String::new();
    let mut binds: Vec<rusqlite::types::Value> = Vec::new();

    // 画师：一个作品只有一个画师，所以多个是「任一」（OR）
    if !artist_uids.is_empty() {
        filter.push_str(&format!(
            " AND artist_uid IN ({})",
            placeholders(artist_uids.len())
        ));
        binds.extend(
            artist_uids
                .iter()
                .map(|uid| rusqlite::types::Value::Text(uid.clone())),
        );
    }

    // 标签：一个作品有很多标签，所以多个是「**同时包含**」（AND）——
    // 多选标签的用途是收窄，OR 只会越选越多。
    // HAVING COUNT(DISTINCT tag) = N 就是「这 N 个标签它都有」。
    if !pixiv_tags.is_empty() {
        filter.push_str(&format!(
            " AND pid IN (
                 SELECT pid FROM illust_tags WHERE tag IN ({})
                 GROUP BY pid HAVING COUNT(DISTINCT tag) = {}
             )",
            placeholders(pixiv_tags.len()),
            pixiv_tags.len()
        ));
        binds.extend(
            pixiv_tags
                .iter()
                .map(|tag| rusqlite::types::Value::Text(tag.clone())),
        );
    }

    if let Some(clause) = rating_clause(rating) {
        filter.push_str(clause);
    }

    (filter, binds)
}

/// 一条标签图库命中的作品条数。侧栏要显示它，所以导出给 `tag_galleries` 用。
pub fn gallery_count(conn: &rusqlite::Connection, tags: &[String], rating: i64) -> AppResult<i64> {
    let (filter, binds) = artwork_filter(&[], tags, rating);
    let count: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM artworks WHERE 1 = 1{filter}"),
        rusqlite::params_from_iter(binds.iter()),
        |row| row.get(0),
    )?;
    Ok(count)
}

/// 把作品移出作品库：删掉数据库记录与对应的缩略图缓存文件。
///
/// **绝不碰原图** —— 这是「移出索引」，不是删除文件。原图始终留在用户自己的
/// 目录里，这也是整个项目的硬约束（应用对图库目录只读不写）。
///
/// 顺带把 `(pid, page)` 记进忽略名单，重扫同一目录不会再收回来 —— 否则
/// 「移除」在用户眼里等于没做。想反悔就用 `clear_ignored_artworks` 清空名单。
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

/// 忽略名单里有多少条 `(pid, page)`。作品库页头用它显示「已忽略 N 条」。
#[tauri::command]
pub async fn count_ignored_artworks(state: State<'_, AppState>) -> AppResult<usize> {
    with_db(&state, |conn| artworks::count_ignored(conn)).await
}

/// 清空忽略名单，返回清掉的条数。之后重新扫描目录会把它们再收回来。
#[tauri::command]
pub async fn clear_ignored_artworks(state: State<'_, AppState>) -> AppResult<usize> {
    with_db(&state, |conn| artworks::clear_ignored(conn)).await
}

/// 列出所有画师及各自的作品数，按总量倒序。
///
/// 画师是**从路径反推**的，不联网 —— pixez 这类下载器落盘时就按
/// `<画师名>_<画师ID>/` 分好了目录。同时统计通用图库与作品库两套索引。
#[tauri::command]
pub async fn list_artists(state: State<'_, AppState>) -> AppResult<Vec<ArtistSummary>> {
    with_db(&state, |conn| crate::artists::list(conn)).await
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

/// 返回当前筛选下**每一张**作品的宽高比，供瀑布流排版使用。
///
/// **一次返回全部**（刻意不分页）：瀑布流的总高取决于「最高那一列有多高」，
/// 拿不到全量就撑不准滚动条，只能边滚边涨。只送 id 与比例时 4430 条约 70KB。
///
/// 首次调用会顺带回填老库缺失的宽高。量文件头**在数据库锁之外**跑 —— 一次要读
/// 几千个文件头，攥着锁会让同期的列表查询白等好几秒（与缩略图那条流水线同一套
/// 原则：不把连接和锁带进线程池）。回填只发生一次，之后纯查表。
#[tauri::command]
pub async fn list_artwork_aspects(
    state: State<'_, AppState>,
    artist_uids: Option<Vec<String>>,
    pixiv_tags: Option<Vec<String>>,
    rating: Option<i64>,
) -> AppResult<Vec<ArtworkAspect>> {
    let artist_uids = clean_filter(artist_uids);
    let pixiv_tags = clean_filter(pixiv_tags);
    let rating = rating.unwrap_or(RATING_ANY);

    let scan = with_db(&state, move |conn| {
        let (filter, binds) = artwork_filter(&artist_uids, &pixiv_tags, rating);
        artworks::scan_aspects(conn, &filter, &binds)
    })
    .await?;

    if scan.pending.is_empty() {
        return Ok(aspects_of(&scan.rows));
    }

    let jobs: Vec<PathBuf> = scan
        .pending
        .iter()
        .map(|(_, _, path)| path.clone())
        .collect();
    let measured =
        tauri::async_runtime::spawn_blocking(move || artworks::measure_dimensions(&jobs))
            .await
            .map_err(|err| AppError::Other(format!("读取图片尺寸的任务失败: {err}")))?;

    // 量到的同时更新内存里那一份 —— 调用方要的就是这份，不必再查一遍库
    let mut rows = scan.rows;
    let mut stored: Vec<(i64, u32, u32)> = Vec::with_capacity(measured.len());
    for ((slot, id, _), dims) in scan.pending.iter().zip(measured.iter()) {
        let Some((width, height)) = dims else { continue };
        rows[*slot].set_size(*width, *height);
        stored.push((*id, *width, *height));
    }

    with_db(&state, move |conn| {
        artworks::store_dimensions(conn, &stored)
    })
    .await?;

    Ok(aspects_of(&rows))
}

fn aspects_of(rows: &[artworks::AspectRow]) -> Vec<ArtworkAspect> {
    rows.iter()
        .map(|row| ArtworkAspect {
            id: row.id,
            aspect: row.aspect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::migrate;

    /// 跑完全部迁移的空库，与 `tag_galleries` 的测试同一套做法。
    fn db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn
    }

    fn add(conn: &rusqlite::Connection, pid: &str, thumbnail: Option<String>) {
        conn.execute(
            "INSERT INTO artworks (pid, original_path, thumbnail_path, page, created_at)
             VALUES (?1, ?2, ?3, 0, 0)",
            params![pid, format!("C:/a/{pid}_p0.jpg"), thumbnail],
        )
        .unwrap();
    }

    /// 走一遍 `list_artworks` 用的同一句 SELECT + 同一个映射函数。
    fn read_all(conn: &rusqlite::Connection) -> Vec<Artwork> {
        conn.prepare(&format!("SELECT {ARTWORK_COLUMNS} FROM artworks ORDER BY id"))
            .unwrap()
            .query_map([], artwork_from_row)
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    /// 改了 `ARTWORK_SIZE` 之后，列表必须把旧尺寸的缩略图路径报成「缺失」，
    /// 前端才会走重生成回退把新尺寸补上。
    ///
    /// 反面（判据写反或漏判）的表现是**静默的**：图能打开、不报错，只是永远糊着 ——
    /// 这正是主人最初只看到「手机上怎么这么模糊」而定位不到的原因，所以钉一条测试。
    #[test]
    fn list_reports_stale_thumbnail_size_as_missing() {
        let conn = db();
        let stale = format!("C:/cache/artworks/{}/1.webp", thumbnails::ARTWORK_SIZE / 2);
        let current = format!("C:/cache/artworks/{}/2.webp", thumbnails::ARTWORK_SIZE);

        add(&conn, "1", Some(stale));
        add(&conn, "2", Some(current.clone()));
        add(&conn, "3", None);

        let items = read_all(&conn);
        assert_eq!(
            items[0].thumbnail_path, None,
            "旧尺寸的路径要让出去，否则主人看到的一直是那张糊的"
        );
        assert_eq!(
            items[1].thumbnail_path.as_deref(),
            Some(current.as_str()),
            "当前尺寸必须原样返回：多一次 IPC 就白丢一次「零 IPC 直接渲染」"
        );
        assert_eq!(items[2].thumbnail_path, None, "库里为 NULL 的保持 NULL");
    }
}

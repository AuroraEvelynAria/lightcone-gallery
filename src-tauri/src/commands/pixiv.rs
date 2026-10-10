//! 按 PID 联网补全作品元数据（标题 / 画师 / 标签）。
//!
//! 端点、字段形状与「为什么不需要 OAuth」见 `crate::pixiv`。
//!
//! ## 限流是这个模块的核心约束，不是附加项
//!
//! 一次批量抓取就是几十上百个请求打到同一个账号的 IP 上，撞限流的代价是主人的
//! 账号被临时封掉。所以这里按开源客户端的实测值来：
//!
//! - **串行 + 每次之间 sleep 800ms**（Pixiv-Archive 的 `SYNC_DELAY_MS` 默认值）
//! - **非 200 一律不重试**，失败写进 `error` 列下次再来
//! - **连续失败到阈值就中止整批**（网络断了 / 代理没开时，继续撞没有任何意义）
//! - **单次调用有上限**，界面要补全整个库就分多次点，而不是一次跑几十分钟
//!
//! 另外：这个端点**不需要也不该带 refresh_token**，所以不存在「认证失败还继续撞」
//! 那条最危险的路 —— 最坏情况就是被限流。

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use rusqlite::OptionalExtension;

use crate::error::{AppError, AppResult};
use crate::pixiv::{self, FetchOutcome};
use crate::state::{AppState, with_db};

/// 元数据抓取进度事件的频道名，与前端 `META_PROGRESS_EVENT` 对应。
const META_PROGRESS_EVENT: &str = "pixiv://meta-progress";

/// 每次请求之间的间隔。见模块注释 —— 这是账号安全的一部分，别调小。
const REQUEST_INTERVAL: Duration = Duration::from_millis(800);

/// 连续失败到这个数就中止整批。单次偶发失败（某条作品被删）不该触发，
/// 所以数的是**连续**失败。
const MAX_CONSECUTIVE_FAILURES: usize = 5;

/// 单次调用的抓取上限。界面分批调用，避免一个请求跑几十分钟还没法中断。
pub const MAX_BATCH: usize = 300;

/// 自动补全每批抓多少。取小一点是为了让进度更新更勤、「停止」响应更快
/// —— 每批结束才检查一次停止标志，批太大最长要等一整个批。
const AUTO_BATCH: usize = 50;

/// 每抓这么多条，额外歇一次。
///
/// 一次跑几十分钟的连续请求，和「点一下、等一会儿、再点一下」的节奏不一样。
/// 加个规律性的停顿更像人在翻页，而不是一个不停歇的脚本。
/// 代价是多花几分钟，换来的是不必赌「IP 会不会被限」。
const COOLDOWN_EVERY: usize = 200;
const AUTO_COOLDOWN: Duration = Duration::from_secs(30);

/// 元数据在 `settings` 表里的键名。
const PROXY_KEY: &str = "pixiv_proxy";

/// 「这一条不用再抓了」的判据，三处查询共用。
///
/// 两种都算不用再抓：**成功取到**（`error IS NULL`）、以及**作品在 Pixiv 上没了**
/// （`missing = 1`，终态）。可重试的失败（网络 / 限流）不算 —— 那种要留在待补全里。
const DONE: &str = "(m.error IS NULL OR m.missing = 1)";

/// 「这一条还要不要抓」的完整谓词。
///
/// **刻意不加 `artist_uid IS NOT NULL`**：曾经加过，但那正好把最该抓的 32 条挡在外面
/// —— 它们的路径里没有画师（文件直接躺在图库根目录下），而接口恰恰能给出画师。
/// 加了它还会让 `pixiv_meta_stats` 的「待补全」比批量实际能抓的多 32，永远归不了零。
/// 三处查询（计数 / 挑选 / 统计）必须用同一个谓词，否则界面的数字和实际行为对不上。
///
/// 用函数而不是 const，是为了让它复用 `DONE` —— 两处各写一遍「什么算抓过了」，
/// 改一处忘一处就会让界面数字和实际行为再次错位。
fn needs_meta() -> String {
    format!("NOT EXISTS (SELECT 1 FROM illust_meta m WHERE m.pid = a.pid AND {DONE})")
}

/// 一批抓取的结果。
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaBatch {
    /// 成功取到并落库的条数
    pub fetched: usize,
    /// 作品不存在 / 已删除的条数
    pub missing: usize,
    /// 其它失败（网络、代理、限流）的条数
    pub failed: usize,
    /// 因为连续失败过多而提前中止
    pub aborted: bool,
    /// 中止的原因是**被限流**（而不是网络不通）。界面要据此提示「过一阵再来」。
    pub rate_limited: bool,
    /// 被用户叫停（不是出错）
    pub stopped: bool,
    /// 整轮自动补全：本轮已经处理了多少条
    pub overall_done: usize,
    /// 整轮自动补全：本轮一共要处理多少条
    pub overall_total: usize,
    /// 整轮实际花了多少秒（单批调用时为 0）
    pub elapsed_secs: u64,
}

/// 元数据抓取的进度负载。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaProgress {
    /// 本批已处理
    pub processed: usize,
    /// 本批总数
    pub total: usize,
    /// 整轮已处理（单批调用时为 0）
    pub overall_done: usize,
    /// 整轮总数（单批调用时为 0）
    pub overall_total: usize,
}

/// 库里还有多少 PID 没成功取过元数据。
#[tauri::command]
pub async fn count_missing_meta(state: State<'_, AppState>) -> AppResult<usize> {
    with_db(&state, |conn| {
        let count: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM artworks a WHERE {}", needs_meta()),
            [],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as usize)
    })
    .await
}

/// 取一批还没成功抓过的 PID，联网补全元数据并落库（单批，供界面手动点）。
///
/// `limit` 会被夹在 `1..=MAX_BATCH`。返回本批的统计；进度通过
/// `pixiv://meta-progress` 事件推送。
#[tauri::command]
pub async fn fetch_missing_meta(
    app: AppHandle,
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> AppResult<MetaBatch> {
    let limit = limit.unwrap_or(100).clamp(1, MAX_BATCH);
    // watch_stop = false：手动单批不看运行标志 —— 那个标志此刻本来就是 false，
    // 照着它判断会让手动批次一条都不抓就退出。
    fetch_batch(&app, &state, limit, 0, 0, false).await
}

/// 自动补全：一批接一批地跑，直到没有待补全的、被叫停、或者撞上限流。
///
/// 存在的理由很实际：待补全两千多条，一次一百条要手动点二十几次。
///
/// ## 为什么不担心把 IP 搞封
///
/// 这一整套的限流参数都是按「宁可慢也不冒险」定的，自动跑并没有放松任何一条：
///
/// - 串行 + 每条之间 `REQUEST_INTERVAL`（800ms）—— 和手动点完全一样
/// - **每 `COOLDOWN_EVERY` 条额外歇 `AUTO_COOLDOWN`**：一次跑几十分钟的连续请求
///   和「点一下等一会儿再点一下」的节奏不一样，加个规律性的停顿更像人在翻页，
///   而不是一个不停歇的脚本
/// - 非 200 一律不重试
/// - **撞到限流信号（429 / 403 / 文案含 Limit）立刻整轮停下**，不是跳过这一条继续
/// - 连续失败 `MAX_CONSECUTIVE_FAILURES` 次也停（网络断了就别白撞）
/// - 随时可以 `stop_meta_fetch` 叫停
///
/// 换句话说：**它不会比手动点更激进**，只是把「点下一次」这件事自动化了。
#[tauri::command]
pub async fn fetch_all_meta(app: AppHandle, state: State<'_, AppState>) -> AppResult<MetaBatch> {
    // 已经在跑就别开第二个 —— 两个循环叠加会把请求速率翻倍，那才是真会出事
    if state.meta_running.swap(true, Ordering::SeqCst) {
        return Err(AppError::InvalidInput(
            "自动补全已经在跑了，别开第二个".to_string(),
        ));
    }

    let started = std::time::Instant::now();
    let mut total = MetaBatch::default();
    let mut since_cooldown = 0usize;

    let result = loop {
        if !state.meta_running.load(Ordering::SeqCst) {
            total.stopped = true;
            break Ok(total);
        }

        // 整轮的总数是动态的：抓完的会被排除，所以每批重新算一次
        let remaining = count_pending(&state).await?;
        if remaining == 0 {
            break Ok(total);
        }
        total.overall_total = total.overall_done + remaining;

        let batch = fetch_batch(
            &app,
            &state,
            AUTO_BATCH.min(remaining),
            total.overall_done,
            total.overall_total,
            true,
        )
        .await?;

        // 一条都没处理（理论上不会发生，保险起见别死循环）
        if batch.fetched + batch.missing + batch.failed == 0 {
            break Ok(total);
        }

        total.fetched += batch.fetched;
        total.missing += batch.missing;
        total.failed += batch.failed;
        total.overall_done += batch.fetched + batch.missing + batch.failed;
        total.rate_limited |= batch.rate_limited;

        // 撞限流 / 连续失败：整轮停，不继续
        if batch.aborted {
            total.aborted = true;
            break Ok(total);
        }

        since_cooldown += batch.fetched + batch.missing + batch.failed;
        if since_cooldown >= COOLDOWN_EVERY {
            since_cooldown = 0;
            // 冷却期间也要能被叫停，所以切成小段睡
            let mut slept = 0u64;
            while slept < AUTO_COOLDOWN.as_secs() {
                if !state.meta_running.load(Ordering::SeqCst) {
                    total.stopped = true;
                    break;
                }
                tokio_sleep(Duration::from_secs(1)).await;
                slept += 1;
            }
            if total.stopped {
                break Ok(total);
            }
        }
    };

    state.meta_running.store(false, Ordering::SeqCst);
    if let Ok(mut summary) = result {
        summary.elapsed_secs = started.elapsed().as_secs();
        Ok(summary)
    } else {
        result
    }
}

/// 叫停自动补全。循环每抓完一批检查一次，所以最多再抓完当前这批就停。
#[tauri::command]
pub async fn stop_meta_fetch(state: State<'_, AppState>) -> AppResult<bool> {
    let was_running = state.meta_running.swap(false, Ordering::SeqCst);
    Ok(was_running)
}

/// 还有多少条待补全。与批量挑选用的是同一个谓词，数字不会对不上。
async fn count_pending(state: &AppState) -> AppResult<usize> {
    with_db(state, |conn| {
        let count: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM artworks a WHERE {}", needs_meta()),
            [],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as usize)
    })
    .await
}

/// 抓一批。`overall_done` / `overall_total` 只在自动补全时有意义，
/// 用来让界面显示整轮进度而不是「本批第几条」。
async fn fetch_batch(
    app: &AppHandle,
    state: &AppState,
    limit: usize,
    overall_done: usize,
    overall_total: usize,
    // 是否响应「停止」。自动补全时为 true，此时**每抓一条就检查一次**，
    // 否则最长要等一整批（50 条 ≈ 50 秒）才停得下来，手感像按钮坏了。
    watch_stop: bool,
) -> AppResult<MetaBatch> {
    let limit = limit.clamp(1, MAX_BATCH);

    // 代理地址与 PID 列表都在阻塞线程里先取出来，网络请求留在异步这边
    let proxy: Option<String> = with_db(state, |conn| {
        Ok(conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [PROXY_KEY],
                |row| row.get::<_, String>(0),
            )
            .ok())
    })
    .await?;

    let pids: Vec<String> = with_db(state, move |conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT DISTINCT a.pid FROM artworks a
             WHERE {}
             ORDER BY CAST(a.pid AS INTEGER) DESC
             LIMIT ?1",
            needs_meta()
        ))?;
        let rows = stmt.query_map([limit as i64], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await?;

    let total = pids.len();
    if total == 0 {
        return Ok(MetaBatch::default());
    }

    let client = pixiv::build_client(proxy.as_deref())?;
    let mut batch = MetaBatch::default();
    let mut consecutive_failures = 0usize;

    for (index, pid) in pids.iter().enumerate() {
        // 每条之前看一眼「停止」。放在这里而不是只在批与批之间看，
        // 是为了让「停止」几乎立刻生效。
        if watch_stop && !state.meta_running.load(Ordering::SeqCst) {
            batch.stopped = true;
            break;
        }

        match pixiv::fetch_illust(&client, pid).await {
            FetchOutcome::Fetched(meta) => {
                consecutive_failures = 0;
                batch.fetched += 1;
                let meta = *meta;
                with_db(state, move |conn| store_meta(conn, &meta)).await?;
            }
            FetchOutcome::Missing => {
                // 作品被删是终态，不算「失败」，也不该触发中止
                consecutive_failures = 0;
                batch.missing += 1;
                let pid = pid.clone();
                with_db(state, move |conn| mark_missing(conn, &pid)).await?;
            }
            FetchOutcome::RateLimited(reason) => {
                // 被限流：**立刻停**，不等连续失败攒够。
                // 继续撞的代价是账号/IP 被更重地限制，而这一批剩下的本来就抓不到了。
                batch.rate_limited = true;
                batch.aborted = true;
                batch.failed += 1;
                let pid = pid.clone();
                let note = format!("被限流，已中止：{reason}");
                with_db(state, move |conn| mark_failed(conn, &pid, &note)).await?;
                let _ = app.emit(
                    META_PROGRESS_EVENT,
                    MetaProgress {
                        processed: index + 1,
                        total,
                        overall_done,
                        overall_total,
                    },
                );
                break;
            }
            FetchOutcome::Failed(reason) => {
                consecutive_failures += 1;
                batch.failed += 1;
                let pid = pid.clone();
                with_db(state, move |conn| mark_failed(conn, &pid, &reason)).await?;

                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    batch.aborted = true;
                    let _ = app.emit(
                        META_PROGRESS_EVENT,
                        MetaProgress {
                            processed: index + 1,
                            total,
                            overall_done,
                            overall_total,
                        },
                    );
                    break;
                }
            }
        }

        let _ = app.emit(
            META_PROGRESS_EVENT,
            MetaProgress {
                processed: index + 1,
                total,
                overall_done,
                overall_total,
            },
        );

        // 最后一条之后不用再等
        if index + 1 < total {
            tokio_sleep(REQUEST_INTERVAL).await;
        }
    }

    Ok(batch)
}

/// 把一条元数据写进 `illust_meta` + `illust_tags`，并把接口给的画师名补到
/// `artists` 上（路径派生的名字被 Windows 清洗过，接口的才是原名）。
fn store_meta(conn: &mut rusqlite::Connection, meta: &pixiv::IllustMeta) -> AppResult<()> {
    let now = crate::db::now_unix();
    let tx = conn.transaction()?;

    tx.execute(
        "INSERT INTO illust_meta
             (pid, title, artist_uid, artist_name, artist_account,
              page_count, x_restrict, sanity_level, illust_type, create_date,
              fetched_at, error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL)
         ON CONFLICT(pid) DO UPDATE SET
             title          = excluded.title,
             artist_uid     = excluded.artist_uid,
             artist_name    = excluded.artist_name,
             artist_account = excluded.artist_account,
             page_count     = excluded.page_count,
             x_restrict     = excluded.x_restrict,
             sanity_level   = excluded.sanity_level,
             illust_type    = excluded.illust_type,
             create_date    = excluded.create_date,
             fetched_at     = excluded.fetched_at,
             error          = NULL,
             missing        = 0",
        rusqlite::params![
            meta.pid,
            meta.title,
            meta.artist_uid,
            meta.artist_name,
            meta.artist_account,
            meta.page_count,
            meta.x_restrict,
            meta.sanity_level,
            meta.illust_type,
            meta.create_date,
            now,
        ],
    )?;

    // 标签整体替换：作品改标签是常事，增量合并会留下已经删掉的旧标签
    tx.execute("DELETE FROM illust_tags WHERE pid = ?1", [&meta.pid])?;
    {
        let mut stmt =
            tx.prepare("INSERT OR IGNORE INTO illust_tags (pid, tag) VALUES (?1, ?2)")?;
        for tag in &meta.tags {
            stmt.execute(rusqlite::params![meta.pid, tag])?;
        }
    }

    // 接口给的是未清洗的名字。路径派生的名字先到先得已经写进去了，
    // 这里只在「接口有名字」时覆盖 —— 接口是权威。
    if !meta.artist_uid.is_empty() && !meta.artist_name.is_empty() {
        tx.execute(
            "INSERT INTO artists (uid, name, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(uid) DO UPDATE SET name = excluded.name",
            rusqlite::params![meta.artist_uid, meta.artist_name, now],
        )?;
    }

    tx.commit()?;
    Ok(())
}

/// 作品不存在：标成**终态**（`missing = 1`），以后不再来问。
///
/// 实测过三个这样的 PID，Pixiv 一律返回 404 —— 作者删了、转私密、或下架了。
/// 本地文件还在，只是 Pixiv 那一侧没了。
fn mark_missing(conn: &mut rusqlite::Connection, pid: &str) -> AppResult<()> {
    let now = crate::db::now_unix();
    conn.execute(
        "INSERT INTO illust_meta (pid, title, fetched_at, error, missing)
         VALUES (?1, '', ?2, '作品在 Pixiv 上不存在或已删除', 1)
         ON CONFLICT(pid) DO UPDATE SET
             fetched_at = excluded.fetched_at,
             error      = excluded.error,
             missing    = 1",
        rusqlite::params![pid, now],
    )?;
    Ok(())
}

/// 抓取失败：把原因记下来。下次按 `error IS NOT NULL` 挑出来重试。
fn mark_failed(conn: &mut rusqlite::Connection, pid: &str, reason: &str) -> AppResult<()> {
    write_error(conn, pid, reason)
}

fn write_error(conn: &mut rusqlite::Connection, pid: &str, reason: &str) -> AppResult<()> {
    let now = crate::db::now_unix();
    conn.execute(
        "INSERT INTO illust_meta (pid, title, fetched_at, error) VALUES (?1, '', ?2, ?3)
         ON CONFLICT(pid) DO UPDATE SET fetched_at = excluded.fetched_at, error = excluded.error",
        rusqlite::params![pid, now, reason],
    )?;
    Ok(())
}

/// 按标签统计，倒序。`limit` 夹在 1..=2000。
#[tauri::command]
pub async fn list_pixiv_tags(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> AppResult<Vec<TagCount>> {
    let limit = limit.unwrap_or(200).clamp(1, 2000);
    with_db(&state, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT t.tag, COUNT(*) AS n
             FROM illust_tags t
             JOIN artworks a ON a.pid = t.pid
             GROUP BY t.tag
             ORDER BY n DESC, t.tag ASC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            let count: i64 = row.get(1)?;
            Ok(TagCount {
                tag: row.get(0)?,
                count: count.max(0) as usize,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await
}

/// 标签 + 用它标过的作品数。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagCount {
    pub tag: String,
    pub count: usize,
}

/// 元数据覆盖情况，给界面显示进度用。
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaStats {
    /// 成功取到元数据的 PID 数
    pub done: usize,
    /// **作品在 Pixiv 上没了**（404，终态，不再重试）
    pub missing: usize,
    /// 抓过但可重试的失败（网络 / 限流）
    pub failed: usize,
    /// 还没抓过、或者失败后等着重试的
    pub pending: usize,
}

#[tauri::command]
pub async fn pixiv_meta_stats(state: State<'_, AppState>) -> AppResult<MetaStats> {
    with_db(&state, |conn| {
        let done: i64 = conn.query_row(
            "SELECT COUNT(*) FROM illust_meta WHERE error IS NULL",
            [],
            |row| row.get(0),
        )?;
        let missing: i64 = conn.query_row(
            "SELECT COUNT(*) FROM illust_meta WHERE missing = 1",
            [],
            |row| row.get(0),
        )?;
        let failed: i64 = conn.query_row(
            "SELECT COUNT(*) FROM illust_meta WHERE error IS NOT NULL AND missing = 0",
            [],
            |row| row.get(0),
        )?;
        // 待补全 = 作品库里没有「成功」也没有「已删除」记录的 PID
        let pending: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM artworks a WHERE {}", needs_meta()),
            [],
            |row| row.get(0),
        )?;
        Ok(MetaStats {
            done: done.max(0) as usize,
            missing: missing.max(0) as usize,
            failed: failed.max(0) as usize,
            pending: pending.max(0) as usize,
        })
    })
    .await
}

/// `tokio::time::sleep` 的薄封装。
///
/// 直接用 `std::thread::sleep` 会占住 async runtime 的工作线程 —— 这个命令是
/// `async fn`，串行 sleep 几十次会把它拖垮。所以必须走 tokio 的定时器。
async fn tokio_sleep(duration: Duration) {
    tokio::time::sleep(duration).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::migrate;
    use crate::pixiv::IllustMeta;
    use rusqlite::Connection;

    fn open_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn
    }

    fn meta() -> IllustMeta {
        IllustMeta {
            pid: "131554746".to_string(),
            title: "カルロッタ水着".to_string(),
            // 接口给的 uid 与路径派生的一致，名字却是未清洗的（带斜杠）
            artist_uid: "58599035".to_string(),
            artist_name: "-273℃/望歩".to_string(),
            artist_account: "walk_273".to_string(),
            page_count: 3,
            x_restrict: 0,
            sanity_level: 2,
            illust_type: 0,
            create_date: "2026-08-01T13:56:00+00:00".to_string(),
            tags: vec!["鳴潮".to_string(), "水着".to_string()],
        }
    }

    fn tags_of(conn: &Connection) -> Vec<String> {
        let mut stmt = conn.prepare("SELECT tag FROM illust_tags ORDER BY tag").unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
    }

    /// 落库要同时写三处：`illust_meta` 一行、`illust_tags` 若干行、以及把接口给的
    /// **未清洗名字**覆盖到 `artists` 上。
    #[test]
    fn store_meta_writes_meta_tags_and_corrects_the_artist_name() {
        let mut conn = open_db();

        // 先造一条路径派生的记录：名字被 Windows 文件名规则清洗过
        conn.execute(
            "INSERT INTO artists (uid, name, created_at) VALUES ('58599035', '-273℃望歩', 0)",
            [],
        )
        .unwrap();

        store_meta(&mut conn, &meta()).unwrap();

        let (title, uid, name, account): (String, String, String, String) = conn
            .query_row(
                "SELECT title, artist_uid, artist_name, artist_account FROM illust_meta",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(title, "カルロッタ水着");
        assert_eq!(uid, "58599035");
        assert_eq!(name, "-273℃/望歩", "接口的名字才是原名");
        assert_eq!(account, "walk_273");

        assert_eq!(tags_of(&conn), vec!["水着".to_string(), "鳴潮".to_string()]);

        // artists 上的名字应当被接口的版本覆盖
        let artist_name: String = conn
            .query_row(
                "SELECT name FROM artists WHERE uid = '58599035'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(artist_name, "-273℃/望歩", "接口是权威，应当覆盖路径派生的名字");
    }

    /// 作品改了标签时，旧标签要被清掉而不是越积越多。
    #[test]
    fn store_meta_replaces_tags_instead_of_merging() {
        let mut conn = open_db();
        store_meta(&mut conn, &meta()).unwrap();

        let mut changed = meta();
        changed.tags = vec!["新标签".to_string()];
        store_meta(&mut conn, &changed).unwrap();

        assert_eq!(tags_of(&conn), vec!["新标签".to_string()], "旧标签应当被清掉");
    }

    /// 失败的抓取也要留痕，否则每次都会从头再撞一遍同一个 PID。
    #[test]
    fn failures_are_recorded_not_dropped() {
        let mut conn = open_db();

        mark_missing(&mut conn, "999999999").unwrap();
        mark_failed(&mut conn, "123", "请求失败: 连接超时").unwrap();

        let rows: Vec<(String, Option<String>, i64)> = {
            let mut stmt = conn
                .prepare("SELECT pid, error, missing FROM illust_meta ORDER BY pid")
                .unwrap();
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert_eq!(rows.len(), 2);

        // 可重试的失败：有 error、missing = 0
        assert_eq!(rows[0].1.as_deref(), Some("请求失败: 连接超时"));
        assert_eq!(rows[0].2, 0, "网络失败是可重试的，不能标成终态");

        // 已删除：终态
        assert_eq!(rows[1].1.as_deref(), Some("作品在 Pixiv 上不存在或已删除"));
        assert_eq!(rows[1].2, 1, "404 是终态");

        // 两者都不算「已补全」：`pixiv_meta_stats` 的 done 认的是 error IS NULL
        let done: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM illust_meta WHERE error IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(done, 0, "失败的行不该被当成已补全");
    }

    /// 「已删除」必须是终态：不能被下一批重新挑走。
    ///
    /// 这条守的是一个真实缺陷 —— 之前 `Missing` 和可重试失败共用 `error` 列，
    /// 挑选条件是「没有 error IS NULL 的行」，于是已删除的作品**每批都会被重新抓**：
    /// 对着永远不会回来的作品反复发请求，而且「待补全」永远归不了零。
    #[test]
    fn missing_works_are_terminal_and_not_retried() {
        let mut conn = open_db();
        conn.execute(
            "INSERT INTO artworks (pid, original_path, page, created_at, artist_uid)
             VALUES ('999999999', 'C:/pics/a_1/999999999_p0.jpg', 0, 0, '1')",
            [],
        )
        .unwrap();

        // 还没抓过 -> 在待补全里
        fn pending(conn: &Connection) -> i64 {
            conn.query_row(
                &format!(
                    "SELECT COUNT(*) FROM artworks a
                     WHERE NOT EXISTS (
                         SELECT 1 FROM illust_meta m WHERE m.pid = a.pid AND {DONE}
                     )"
                ),
                [],
                |row| row.get(0),
            )
            .unwrap()
        }
        assert_eq!(pending(&conn), 1, "没抓过的应当待补全");

        mark_missing(&mut conn, "999999999").unwrap();
        assert_eq!(pending(&conn), 0, "已删除的**不该**再待补全 —— 否则每批都重抓");

        // 可重试的失败则相反：要留在待补全里等下次
        let mut conn2 = open_db();
        conn2
            .execute(
                "INSERT INTO artworks (pid, original_path, page, created_at, artist_uid)
                 VALUES ('888', 'C:/pics/a_1/888_p0.jpg', 0, 0, '1')",
                [],
            )
            .unwrap();
        mark_failed(&mut conn2, "888", "请求失败: 连接超时").unwrap();
        assert_eq!(pending(&conn2), 1, "网络失败要留在待补全里等重试");
    }

    /// 路径里没有画师的作品**也要**进待补全 —— 它们恰恰最需要元数据。
    ///
    /// 这条守的是一个真实缺陷：挑选 SQL 里曾经带着 `artist_uid IS NOT NULL`，
    /// 正好把文件直接躺在图库根目录下、路径推不出画师的 32 条挡在外面。
    /// 而接口是能给出画师的 —— 挡掉它们等于永远修不好这 32 条。
    /// 顺带那个条件还让界面的「待补全」比批量实际能抓的多 32，永远归不了零。
    #[test]
    fn works_without_a_path_artist_are_still_queued() {
        let conn = open_db();
        // artist_uid 为 NULL：路径里推不出画师
        conn.execute(
            "INSERT INTO artworks (pid, original_path, page, created_at, artist_uid)
             VALUES ('777', 'C:/pics/777_p0.jpg', 0, 0, NULL)",
            [],
        )
        .unwrap();

        let pending: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM artworks a WHERE {}", needs_meta()),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending, 1, "没有画师的作品也该被排队，接口能补上画师");

        // 取到元数据之后就该出队
        conn.execute(
            "INSERT INTO illust_meta (pid, title, artist_uid, artist_name, fetched_at)
             VALUES ('777', 't', '1', 'a', 0)",
            [],
        )
        .unwrap();
        let after: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM artworks a WHERE {}", needs_meta()),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(after, 0);
    }

    /// 「终态」不该是单向门：清掉标记后能重新进入待补全。
    #[test]
    fn resetting_missing_puts_them_back_in_the_queue() {
        let mut conn = open_db();
        mark_missing(&mut conn, "999999999").unwrap();

        let cleared = conn
            .execute("DELETE FROM illust_meta WHERE missing = 1", [])
            .unwrap();
        assert_eq!(cleared, 1);

        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM illust_meta", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 0, "清掉之后应当没有任何记录，等于「没抓过」");
    }

    /// 成功后重抓（比如上次失败过）应当把 error 清掉。
    #[test]
    fn a_successful_refetch_clears_the_previous_error() {
        let mut conn = open_db();
        mark_failed(&mut conn, "131554746", "上次超时").unwrap();
        store_meta(&mut conn, &meta()).unwrap();

        let error: Option<String> = conn
            .query_row("SELECT error FROM illust_meta", [], |row| row.get(0))
            .unwrap();
        assert_eq!(error, None, "重抓成功应当清掉 error");
    }
}

/// 取一条作品的 Pixiv 侧信息（标题 / 画师 / 标签），供检视弹窗显示。
///
/// 返回 `None` 表示这条作品还没补全过元数据 —— 前端据此显示「还没取过」，
/// 而不是显示成「没有标签」。
#[tauri::command]
pub async fn illust_detail(
    state: State<'_, AppState>,
    pid: String,
) -> AppResult<Option<crate::db::models::IllustDetailView>> {
    use crate::db::models::IllustDetailView;

    with_db(&state, move |conn| {
        let row = conn
            .query_row(
                "SELECT pid, title, artist_uid, artist_name, artist_account,
                        page_count, x_restrict, illust_type, create_date
                 FROM illust_meta WHERE pid = ?1 AND error IS NULL",
                [&pid],
                |row| {
                    Ok(IllustDetailView {
                        pid: row.get(0)?,
                        title: row.get(1)?,
                        artist_uid: row.get(2)?,
                        artist_name: row.get(3)?,
                        artist_account: row.get(4)?,
                        page_count: row.get(5)?,
                        x_restrict: row.get(6)?,
                        illust_type: row.get(7)?,
                        create_date: row.get(8)?,
                        tags: Vec::new(),
                    })
                },
            )
            .optional()?;

        let Some(mut detail) = row else {
            return Ok(None);
        };

        let mut stmt =
            conn.prepare("SELECT tag FROM illust_tags WHERE pid = ?1 ORDER BY tag ASC")?;
        let tags = stmt.query_map([&pid], |row| row.get::<_, String>(0))?;
        detail.tags = tags.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some(detail))
    })
    .await
}

/// 把「已删除」标记清掉，让它们重新进入待补全。
///
/// 存在的理由：作品转私密之后再放出来是常事，Pixiv 对这两种情况的 404 无法区分。
/// 所以「终态」不该是单向门 —— 主人想重试时得有个开关。
#[tauri::command]
pub async fn reset_missing_meta(state: State<'_, AppState>) -> AppResult<usize> {
    with_db(&state, |conn| {
        let cleared = conn.execute(
            "DELETE FROM illust_meta WHERE missing = 1",
            [],
        )?;
        Ok(cleared)
    })
    .await
}

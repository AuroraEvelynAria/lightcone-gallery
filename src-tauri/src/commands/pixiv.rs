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

use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use rusqlite::OptionalExtension;

use crate::error::AppResult;
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

/// 元数据在 `settings` 表里的键名。
const PROXY_KEY: &str = "pixiv_proxy";

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
}

/// 元数据抓取的进度负载。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaProgress {
    pub processed: usize,
    pub total: usize,
}

/// 库里还有多少 PID 没成功取过元数据。
#[tauri::command]
pub async fn count_missing_meta(state: State<'_, AppState>) -> AppResult<usize> {
    with_db(&state, |conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM artworks a
             WHERE a.artist_uid IS NOT NULL
               AND NOT EXISTS (
                   SELECT 1 FROM illust_meta m WHERE m.pid = a.pid AND m.error IS NULL
               )",
            [],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as usize)
    })
    .await
}

/// 取一批还没成功抓过的 PID，联网补全元数据并落库。
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

    // 代理地址与 PID 列表都在阻塞线程里先取出来，网络请求留在异步这边
    let proxy: Option<String> = with_db(&state, |conn| {
        Ok(conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [PROXY_KEY],
                |row| row.get::<_, String>(0),
            )
            .ok())
    })
    .await?;

    let pids: Vec<String> = with_db(&state, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT a.pid FROM artworks a
             WHERE a.artist_uid IS NOT NULL
               AND NOT EXISTS (
                   SELECT 1 FROM illust_meta m WHERE m.pid = a.pid AND m.error IS NULL
               )
             ORDER BY CAST(a.pid AS INTEGER) DESC
             LIMIT ?1",
        )?;
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
        match pixiv::fetch_illust(&client, pid).await {
            FetchOutcome::Fetched(meta) => {
                consecutive_failures = 0;
                batch.fetched += 1;
                let meta = *meta;
                with_db(&state, move |conn| store_meta(conn, &meta)).await?;
            }
            FetchOutcome::Missing => {
                // 作品被删是终态，不算「失败」，也不该触发中止
                consecutive_failures = 0;
                batch.missing += 1;
                let pid = pid.clone();
                with_db(&state, move |conn| mark_missing(conn, &pid)).await?;
            }
            FetchOutcome::RateLimited(reason) => {
                // 被限流：**立刻停**，不等连续失败攒够。
                // 继续撞的代价是账号/IP 被更重地限制，而这一批剩下的本来就抓不到了。
                batch.rate_limited = true;
                batch.aborted = true;
                batch.failed += 1;
                let pid = pid.clone();
                let note = format!("被限流，已中止：{reason}");
                with_db(&state, move |conn| mark_failed(conn, &pid, &note)).await?;
                let _ = app.emit(
                    META_PROGRESS_EVENT,
                    MetaProgress {
                        processed: index + 1,
                        total,
                    },
                );
                break;
            }
            FetchOutcome::Failed(reason) => {
                consecutive_failures += 1;
                batch.failed += 1;
                let pid = pid.clone();
                with_db(&state, move |conn| mark_failed(conn, &pid, &reason)).await?;

                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    batch.aborted = true;
                    let _ = app.emit(
                        META_PROGRESS_EVENT,
                        MetaProgress {
                            processed: index + 1,
                            total,
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
             error          = NULL",
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

/// 作品不存在（已删除）：记一条带 error 的行，避免以后反复来问。
fn mark_missing(conn: &mut rusqlite::Connection, pid: &str) -> AppResult<()> {
    write_error(conn, pid, "作品不存在或已删除")
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
    /// 有元数据的 PID 数（成功的）
    pub done: usize,
    /// 抓过但失败的（含已删除）
    pub failed: usize,
    /// 还没抓过的
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
        let failed: i64 = conn.query_row(
            "SELECT COUNT(*) FROM illust_meta WHERE error IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        let pending: i64 = conn.query_row(
            "SELECT COUNT(*) FROM artworks a
             WHERE NOT EXISTS (SELECT 1 FROM illust_meta m WHERE m.pid = a.pid)",
            [],
            |row| row.get(0),
        )?;
        Ok(MetaStats {
            done: done.max(0) as usize,
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

        let rows: Vec<(String, Option<String>)> = {
            let mut stmt = conn.prepare("SELECT pid, error FROM illust_meta ORDER BY pid").unwrap();
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].1.as_deref(), Some("请求失败: 连接超时"));
        assert_eq!(rows[1].1.as_deref(), Some("作品不存在或已删除"));

        // 失败的行不算「已补全」：`count_missing_meta` 认的是 error IS NULL
        let done: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM illust_meta WHERE error IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(done, 0, "失败的行不该被当成已补全");
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

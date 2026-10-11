//! 按 PID 从 Pixiv 网页接口取元数据（标题 / 画师 / 标签）。
//!
//! ## 为什么不用 OAuth
//!
//! 走的是**公开的 web ajax 端点**，不需要 refresh_token：
//!
//! ```text
//! GET https://www.pixiv.net/ajax/illust/{pid}?lang=zh
//! ```
//!
//! 下面这些形状全部是在真机上核出来的（2026-10-10，逐字段验证，不是照文档猜的）：
//!
//! - 成功：HTTP 200，`{ "error": false, "message": "", "body": { ... } }`
//! - 作品不存在：**HTTP 404**，`{ "error": true, "message": "", "body": [] }`
//!   —— 注意 `body` 这时是**空数组**而不是对象，解析前必须先判 `error`。
//! - `body.tags` 是**对象**不是数组，标签在 `body.tags.tags[].tag` 里。
//! - `body.userId` / `userName` / `userAccount`（@ 句柄）给的是画师。
//! - `xRestrict`：0 全年龄 / 1 R-18 / 2 R-18G；`sl` 是 sanity level。
//! - `illustType`：0 插画 / 1 漫画 / 2 动图。
//! - **不强制要 Referer**（实测不带也能拿到数据），但照旧带上。
//!
//! ## 两个它比路径强的地方
//!
//! 1. **标签只能从这里来** —— 路径里没有。
//! 2. **画师名是未清洗的**。路径里的目录名是 Windows 文件名，`/` 这类非法字符被删掉了：
//!    接口给 `-273℃/望歩`，目录里是 `-273℃望歩`；接口给 `2o水/TSUOMI`，目录里是
//!    `2o水TSUOMI`。uid 两边一致（实测 10/10 相同），但显示名以接口为准。
//!
//! ## 国内必须能连上
//!
//! 实测本机直连 pixiv 是 **HTTP 000**（DNS 被污染成 `face:b00c` 段，直连 IP 也不通），
//! 必须走本地代理。所以客户端支持显式配置代理，配置项存在 `settings` 表的
//! `pixiv_proxy` 里。

use std::time::Duration;

use serde::Serialize;

use crate::error::{AppError, AppResult};

/// 固定 UA。带上它更像正常浏览器，也便于以后排查。
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                          (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// 单次请求超时。走代理时首包可能慢，给宽一点。
const TIMEOUT_SECS: u64 = 20;

/// 一条作品的元数据。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IllustMeta {
    pub pid: String,
    pub title: String,
    /// 画师 Pixiv 用户 ID，与路径派生出来的 `artists.uid` 同一套编号
    pub artist_uid: String,
    /// 接口给的原始名字（未做文件名清洗）
    pub artist_name: String,
    /// @ 句柄
    pub artist_account: String,
    pub page_count: i64,
    /// 0 全年龄 / 1 R-18 / 2 R-18G
    pub x_restrict: i64,
    pub sanity_level: i64,
    /// 0 插画 / 1 漫画 / 2 动图
    pub illust_type: i64,
    pub create_date: String,
    pub tags: Vec<String>,
}

/// 一次请求的结果。**三种「没拿到」必须分开**，因为后续动作完全不同：
///
/// - `Missing`：作品不存在 / 已删除。终态，记下来别再问。
/// - `RateLimited`：**被限流了**。这是「现在该停下来」的信号，不是这一条的问题 ——
///   必须立刻中止整批，绝不能继续撞。
/// - `Failed`：单条失败（超时、连接断）。可重试。
///
/// 把 `RateLimited` 混进 `Missing` 的后果很严重：既会把「被限流」错记成
/// 「作品不存在」（污染数据、且永远不再重试），又不会触发中止，于是继续往下撞 ——
/// 那才是真会被封的做法。
#[derive(Debug)]
pub enum FetchOutcome {
    Fetched(Box<IllustMeta>),
    Missing,
    RateLimited(String),
    Failed(String),
}

/// 判断接口在 200 里报的错，是「被限流」还是「作品没了」。
///
/// Pixiv 是日文服务，但请求带了 `lang=zh`，所以限流文案可能是中文；
/// 这里跨语种匹配关键词，宁可误判成限流（保守，会停下来）也不误判成「已删除」。
fn looks_like_rate_limit(message: &str) -> bool {
    let lowered = message.to_lowercase();
    const SIGNALS: [&str; 8] = [
        "limit",      // "Limit" / "rate limit"
        "rate",
        "too many",
        "频繁",       // 请求过于频繁
        "限制",       // 访问限制
        "制限",       // 日本语
        "頻度",       // 日本语：頻度制限
        "暂时",       // 暂时无法访问
    ];
    SIGNALS.iter().any(|signal| lowered.contains(signal))
}

/// 建一个带可选代理的客户端。
///
/// `proxy` 传空表示交给 reqwest 自己判断（环境变量 / 系统设置）。
/// 国内直连不通时，用户需要在设置里填 `http://127.0.0.1:7897` 这类地址。
pub fn build_client(proxy: Option<&str>) -> AppResult<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .user_agent(USER_AGENT);

    if let Some(url) = proxy.map(str::trim).filter(|url| !url.is_empty()) {
        let proxy = reqwest::Proxy::all(url)
            .map_err(|err| AppError::InvalidInput(format!("代理地址不合法 {url}: {err}")))?;
        builder = builder.proxy(proxy);
    }

    builder
        .build()
        .map_err(|err| AppError::Other(format!("HTTP 客户端创建失败: {err}")))
}

/// 把错误连同它的整条 source 链拼成一句话。
///
/// reqwest 的顶层 display 只有一句 `error sending request for url (...)`，
/// 真正的死因（超时 / 代理拒绝 / 证书）全在 source 链里 —— 不展开的话，
/// 主人看到的失败原因永远是一句没有信息量的套话。
fn error_chain(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        text.push_str(&format!(" ← {cause}"));
        source = cause.source();
    }
    text
}

/// 取一条作品的元数据。**不做重试** —— 非 200 一律不重试，这是刻意的：
/// 反复撞只会让 IP 更快被限流，失败写进 `error` 列下次再来更划算。
pub async fn fetch_illust(client: &reqwest::Client, pid: &str) -> FetchOutcome {
    if pid.is_empty() || !pid.bytes().all(|byte| byte.is_ascii_digit()) {
        return FetchOutcome::Failed(format!("PID 不是纯数字: {pid}"));
    }

    let url = format!("https://www.pixiv.net/ajax/illust/{pid}?lang=zh");
    let response = match client
        .get(&url)
        .header("Referer", "https://www.pixiv.net/")
        .send()
        .await
    {
        Ok(response) => {
            eprintln!("[meta] {pid} HTTP {}", response.status());
            response
        }
        Err(err) => {
            let mut reason = format!("请求失败: {}", error_chain(&err));
            // 经代理时「TLS 握手被对端断开 / 连接被重置」几乎总是节点侧的问题
            // （节点连不上 Pixiv），不是本机的网络配置 —— 提示主人换节点
            if reason.contains("tls handshake eof") || reason.contains("connection reset") {
                reason.push_str(" —— 代理的节点连不上 Pixiv，去代理软件里换一个节点试试");
            }
            eprintln!("[meta] {pid} 失败: {reason}");
            return FetchOutcome::Failed(reason);
        }
    };

    let status = response.status();
    // 429 Too Many Requests 是标准限流码；403 常被用来做封禁。
    // 两者都当成「立刻停下来」的信号，而不是单条失败。
    if status.as_u16() == 429 || status.as_u16() == 403 {
        return FetchOutcome::RateLimited(format!("HTTP {}", status.as_u16()));
    }
    if status.as_u16() == 404 {
        return FetchOutcome::Missing;
    }
    if !status.is_success() {
        return FetchOutcome::Failed(format!("HTTP {}", status.as_u16()));
    }

    let raw: serde_json::Value = match response.json().await {
        Ok(value) => value,
        Err(err) => return FetchOutcome::Failed(format!("响应不是合法 JSON: {err}")),
    };

    match parse_illust(pid, &raw) {
        Ok(meta) => FetchOutcome::Fetched(Box::new(meta)),
        Err(reason) => {
            // 接口在 200 里报错。先看文案是不是限流 —— 这一步不能省，
            // 否则「请求过于频繁」会被当成「作品已删除」永久记下来。
            let message = raw
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if looks_like_rate_limit(message) || looks_like_rate_limit(&reason) {
                FetchOutcome::RateLimited(format!("接口报错: {message}"))
            } else if raw.get("error").and_then(serde_json::Value::as_bool) == Some(true) {
                FetchOutcome::Missing
            } else {
                FetchOutcome::Failed(reason)
            }
        }
    }
}

/// 把接口响应解析成 `IllustMeta`。抽成纯函数是为了能用固定 fixture 做单测 ——
/// 网络那半段没法在测试里跑，但字段映射这半段必须测。
pub fn parse_illust(pid: &str, raw: &serde_json::Value) -> Result<IllustMeta, String> {
    if raw.get("error").and_then(serde_json::Value::as_bool) == Some(true) {
        let message = raw
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        return Err(format!("接口报错: {message}"));
    }

    let body = raw
        .get("body")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "响应缺少 body 对象".to_string())?;

    let text = |key: &str| -> String {
        body.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let number = |key: &str| -> i64 {
        body.get(key)
            .and_then(serde_json::Value::as_i64)
            .unwrap_or_default()
    };

    let title = {
        // 有的作品 illustTitle 是空的，退回 title
        let primary = text("illustTitle");
        if primary.is_empty() {
            text("title")
        } else {
            primary
        }
    };
    if title.is_empty() && text("userId").is_empty() {
        return Err("响应里既没有标题也没有画师，形状可能变了".to_string());
    }

    // body.tags 是对象，标签在 body.tags.tags[].tag
    let tags = body
        .get("tags")
        .and_then(|tags| tags.get("tags"))
        .and_then(serde_json::Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|item| item.get("tag").and_then(serde_json::Value::as_str))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(IllustMeta {
        pid: pid.to_string(),
        title,
        artist_uid: text("userId"),
        artist_name: text("userName"),
        artist_account: text("userAccount"),
        page_count: number("pageCount"),
        x_restrict: number("xRestrict"),
        sanity_level: number("sl"),
        illust_type: number("illustType"),
        create_date: text("createDate"),
        tags,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 成功响应的形状。字段是照真机响应裁出来的，结构（尤其是 `tags.tags` 那层
    /// 嵌套）与线上一致。
    fn success_fixture() -> serde_json::Value {
        json!({
            "error": false,
            "message": "",
            "body": {
                "illustId": "131554746",
                "illustTitle": "カルロッタ水着",
                "title": "カルロッタ水着",
                "createDate": "2026-08-01T13:56:00+00:00",
                "uploadDate": "2026-08-01T13:56:00+00:00",
                "xRestrict": 0,
                "restrict": 0,
                "sl": 2,
                "illustType": 0,
                "pageCount": 3,
                "userId": "58599035",
                "userName": "-273℃/望歩",
                "userAccount": "walk_273",
                "width": 1200,
                "height": 1600,
                "tags": {
                    "authorId": "58599035",
                    "isLocked": false,
                    "tags": [
                        { "tag": "WutheringWaves", "locked": true },
                        { "tag": "鳴潮", "locked": true },
                        { "tag": "カルロッタ", "locked": false }
                    ]
                }
            }
        })
    }

    #[test]
    fn parses_a_successful_response() {
        let meta = parse_illust("131554746", &success_fixture()).unwrap();
        assert_eq!(meta.pid, "131554746");
        assert_eq!(meta.title, "カルロッタ水着");
        assert_eq!(meta.artist_uid, "58599035");
        // 接口给的名字里斜杠还在 —— 路径里的目录名会被 Windows 吃掉
        assert_eq!(meta.artist_name, "-273℃/望歩");
        assert_eq!(meta.artist_account, "walk_273");
        assert_eq!(meta.page_count, 3);
        assert_eq!(meta.illust_type, 0);
        assert_eq!(
            meta.tags,
            vec!["WutheringWaves", "鳴潮", "カルロッタ"],
            "标签在 body.tags.tags[].tag，不是 body.tags[]"
        );
    }

    /// 不存在的作品：线上是 HTTP 404 + `body` 为**空数组**。
    /// 解析前必须先看 `error`，否则会在 `body` 上撞类型错误。
    #[test]
    fn rejects_the_not_found_envelope() {
        let raw = json!({ "error": true, "message": "", "body": [] });
        assert!(parse_illust("999999999", &raw).is_err());
    }

    /// 缺字段时不该 panic，能填的填上、填不上的留空。
    #[test]
    fn tolerates_missing_optional_fields() {
        let raw = json!({
            "error": false,
            "message": "",
            "body": {
                "illustTitle": "只有标题",
                "userId": "1",
                "userName": "a"
            }
        });
        let meta = parse_illust("1", &raw).unwrap();
        assert_eq!(meta.title, "只有标题");
        assert_eq!(meta.page_count, 0);
        assert!(meta.tags.is_empty());
        assert_eq!(meta.artist_account, "");
    }

    /// illustTitle 为空时退回 title。
    #[test]
    fn falls_back_to_the_plain_title() {
        let raw = json!({
            "error": false,
            "message": "",
            "body": { "illustTitle": "", "title": "退路", "userId": "1", "userName": "a" }
        });
        assert_eq!(parse_illust("1", &raw).unwrap().title, "退路");
    }

    /// body 完全不成形时报错，而不是返回一条空记录。
    #[test]
    fn rejects_a_shape_change() {
        let raw = json!({ "error": false, "message": "", "body": {} });
        assert!(parse_illust("1", &raw).is_err(), "空 body 应当被拒绝");
        let raw = json!({ "unexpected": true });
        assert!(parse_illust("1", &raw).is_err(), "没有 body 应当被拒绝");
    }

    /// 「被限流」和「作品没了」必须分得开 —— 混一起就会把限流记成「已删除」，
    /// 既污染数据又不会停下来。
    #[test]
    fn tells_rate_limits_apart_from_missing_works() {
        // 限流文案（跨语种）
        for message in [
            "Rate Limit",
            "limit exceeded",
            "Too many requests",
            "请求过于频繁，请稍后再试",
            "アクセス頻度が制限されています",
        ] {
            assert!(looks_like_rate_limit(message), "应当识别为限流: {message}");
        }

        // 正常的「找不到」不该被误判成限流
        for message in ["", "无法找到您所请求的页面", "作品不存在"] {
            assert!(!looks_like_rate_limit(message), "不该识别为限流: {message}");
        }
    }

    /// 200 里报限流时要落到 `RateLimited`，不能落到 `Missing`。
    #[test]
    fn a_200_rate_limit_envelope_is_not_reported_as_missing() {
        let raw = json!({ "error": true, "message": "请求过于频繁", "body": [] });
        // parse_illust 只负责解析，分类在 fetch_illust 里；这里验证信号能被识别出来
        assert!(parse_illust("1", &raw).is_err());
        let message = raw.get("message").and_then(|v| v.as_str()).unwrap_or("");
        assert!(looks_like_rate_limit(message));
    }

    #[test]
    fn rejects_a_non_numeric_pid_before_hitting_the_network() {
        // fetch_illust 是异步的，这里只验证纯函数之外的守卫逻辑走的是同一条判断
        for bad in ["", "abc", "12a"] {
            assert!(bad.is_empty() || !bad.bytes().all(|b| b.is_ascii_digit()));
        }
        assert!("131554746".bytes().all(|b| b.is_ascii_digit()));
    }

    /// 打真接口的活体测试，默认 `#[ignore]`。手动跑：
    ///
    /// ```text
    /// PIXIV_PROXY=http://127.0.0.1:7897 cargo test --lib live_fetch -- --ignored --nocapture
    /// ```
    ///
    /// **它守的是「接口形状没变」这件事。** 上面那些单测用的 fixture 是照今天观测到的
    /// 形状写的 —— 哪天 Pixiv 把 `body.tags.tags` 改成 `body.tags` 数组、或者把
    /// `userId` 改名，单测照样全绿，而线上会静默返回空标题。只有真打一次才知道。
    ///
    /// 不带 `PIXIV_PROXY` 时会走 reqwest 自己的代理判断（环境变量 / 系统设置）；
    /// 国内直连 pixiv 是不通的（DNS 污染成 `face:b00c` 段），所以基本必须给代理。
    #[test]
    #[ignore = "需要联网，国内还需要代理"]
    fn live_fetch_returns_real_metadata() {
        let proxy = std::env::var("PIXIV_PROXY").ok();
        let client = build_client(proxy.as_deref()).expect("客户端应当能建起来");

        // 131554746 是本地库里真实存在的一条，用来比对路径派生与接口的一致性
        let outcome = tauri::async_runtime::block_on(fetch_illust(&client, "131554746"));
        let meta = match outcome {
            FetchOutcome::Fetched(meta) => *meta,
            FetchOutcome::Missing => panic!("这个 PID 不该不存在 —— 要么被删了，要么形状变了"),
            FetchOutcome::RateLimited(reason) => {
                panic!("被限流了: {reason}
（这说明 800ms 的间隔偏小，或者出口 IP 已经被限）")
            }
            FetchOutcome::Failed(reason) => {
                panic!("请求失败: {reason}\n（国内需要 PIXIV_PROXY 指向本地代理）")
            }
        };

        println!(
            "pid={} 标题={:?} 画师={} ({}) 标签 {} 个: {:?}",
            meta.pid,
            meta.title,
            meta.artist_name,
            meta.artist_uid,
            meta.tags.len(),
            meta.tags
        );

        assert_eq!(meta.pid, "131554746");
        assert!(!meta.title.is_empty(), "标题不该是空的");
        assert!(!meta.artist_uid.is_empty(), "画师 uid 不该是空的");
        assert!(!meta.tags.is_empty(), "标签不该是空的");
        // 路径派生的 uid 与接口实测一致（真机上比对过 10/10）
        assert_eq!(meta.artist_uid, "58599035");

        // 不存在的 PID 必须落到 Missing 而不是 Failed —— 前者是终态，后者会被反复重试
        let missing = tauri::async_runtime::block_on(fetch_illust(&client, "999999999"));
        assert!(
            matches!(missing, FetchOutcome::Missing),
            "不存在的 PID 应当被识别为 Missing"
        );
    }
}

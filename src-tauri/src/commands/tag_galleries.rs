//! 标签图库：侧栏里「一组标签 + 一个分级」的独立图库。
//!
//! 存在的理由是把 NSFW 从日常浏览里分出去。**内置的「全年龄 / R-18」两条不落库**
//! —— 它们就是 `rating` 这一个维度，前端直接拼出来即可，落库反而要处理
//! 「用户把它删了怎么办」。这里只管用户自建的那些。
//!
//! 命中条数一律**实时算**（`artworks::gallery_count`），不存列：作品库随时会
//! 增删、元数据也随时在补，存下来的计数必然过期，而侧栏上的数字一旦对不上
//! 就会让人怀疑整个筛选是不是坏的。

use rusqlite::params;
use tauri::State;

use crate::commands::artworks::gallery_count;
use crate::db::models::{RATING_ALL_AGES, RATING_ANY, RATING_NSFW, TagGallery};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, with_db};

/// 名字长度上限（按字符算）。侧栏一行放不下更长的，早点拒绝比界面上截断好。
const MAX_NAME_CHARS: usize = 40;
/// 一条图库最多绑多少个标签。标签是「同时包含」语义，叠太多必然筛空。
const MAX_TAGS: usize = 12;

/// 按分级数出来的三个数。侧栏那两条内置图库要显示条数，而它们不落库，
/// 所以不能走 `tag_galleries` 那条路。
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RatingCounts {
    /// `x_restrict = 0`
    pub all_ages: i64,
    /// `x_restrict >= 1`（R-18 与 R-18G）
    pub nsfw: i64,
    /// **没抓过元数据**的作品：既不是全年龄也不是 R-18。
    /// 界面上选了分级之后要单独提示它，否则主人会以为作品丢了。
    pub unknown: i64,
}

/// 分级计数的 SQL。抽成独立函数是为了能脱离 `State` 直接测（见文件末尾的测试）。
pub fn rating_counts(conn: &rusqlite::Connection) -> AppResult<RatingCounts> {
    let (all_ages, nsfw, unknown) = conn.query_row(
        "SELECT
             COUNT(*) FILTER (WHERE pid IN (SELECT pid FROM illust_meta WHERE x_restrict = 0)),
             COUNT(*) FILTER (WHERE pid IN (SELECT pid FROM illust_meta WHERE x_restrict >= 1)),
             COUNT(*) FILTER (WHERE pid NOT IN (SELECT pid FROM illust_meta))
         FROM artworks",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    Ok(RatingCounts {
        all_ages,
        nsfw,
        unknown,
    })
}

/// 数一数各分级下有多少条作品。一次查询搞定，别让前端为三个数字跑三趟。
#[tauri::command]
pub async fn count_artworks_by_rating(state: State<'_, AppState>) -> AppResult<RatingCounts> {
    with_db(&state, |conn| rating_counts(conn)).await
}

/// 把库里存的 JSON 数组读成标签列表。
///
/// 解析失败一律当空列表 —— 这是本地库，宁可少筛也不要因为一条坏记录
/// 让整个侧栏读不出来。
fn parse_tags(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

/// 规范化标签：去空白、去重、限个数。
fn clean_tags(values: Vec<String>) -> Vec<String> {
    let mut cleaned: Vec<String> = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    cleaned.dedup();
    cleaned.truncate(MAX_TAGS);
    cleaned
}

/// 读出全部自建图库，每条带上实时命中的作品条数。
#[tauri::command]
pub async fn list_tag_galleries(state: State<'_, AppState>) -> AppResult<Vec<TagGallery>> {
    with_db(&state, |conn| {
        // 先把行收完再算计数：stmt 会一直借着 conn，边遍历边查容易踩借用的坑
        let rows: Vec<(i64, String, String, i64, i64)> = {
            let mut stmt = conn.prepare(
                "SELECT id, name, tags, rating, created_at
                 FROM tag_galleries ORDER BY created_at ASC, id ASC",
            )?;
            let mapped = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let mut out = Vec::with_capacity(rows.len());
        for (id, name, tags_json, rating, created_at) in rows {
            let tags = parse_tags(&tags_json);
            let count = gallery_count(conn, &tags, rating)?;
            out.push(TagGallery {
                id,
                name,
                tags,
                rating,
                created_at,
                count,
            });
        }
        Ok(out)
    })
    .await
}

/// 新建一条标签图库。
#[tauri::command]
pub async fn create_tag_gallery(
    state: State<'_, AppState>,
    name: String,
    tags: Vec<String>,
    rating: i64,
) -> AppResult<TagGallery> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::InvalidInput("图库名字不能为空".into()));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(AppError::InvalidInput(format!(
            "图库名字最多 {MAX_NAME_CHARS} 个字"
        )));
    }
    // 名字重复不算错，但要挡住「连点两次建出两条一模一样的」——
    // 侧栏里两条同名项谁也分不清哪个是哪个
    let tags = clean_tags(tags);
    if tags.is_empty() && rating == RATING_ANY {
        return Err(AppError::InvalidInput(
            "至少选一个标签，或者选一个分级 —— 否则这条图库就是「全部作品」".into(),
        ));
    }
    if !matches!(rating, RATING_ANY | RATING_ALL_AGES | RATING_NSFW) {
        return Err(AppError::InvalidInput(format!("不认识的分级取值: {rating}")));
    }

    with_db(&state, move |conn| {
        let duplicate: Option<i64> = conn
            .query_row(
                "SELECT id FROM tag_galleries WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .ok();
        if let Some(id) = duplicate {
            return Err(AppError::InvalidInput(format!(
                "已经有一条叫「{name}」的图库了（id {id}）"
            )));
        }

        let tags_json = serde_json::to_string(&tags)
            .map_err(|err| AppError::Other(format!("标签序列化失败: {err}")))?;
        let created_at = crate::db::now_unix();

        conn.execute(
            "INSERT INTO tag_galleries (name, tags, rating, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![name, tags_json, rating, created_at],
        )?;
        let id = conn.last_insert_rowid();
        let count = gallery_count(conn, &tags, rating)?;

        Ok(TagGallery {
            id,
            name,
            tags,
            rating,
            created_at,
            count,
        })
    })
    .await
}

/// 删掉一条自建图库。只是去掉侧栏里的一个入口，**作品本身一条都不动**。
#[tauri::command]
pub async fn delete_tag_gallery(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    with_db(&state, move |conn| {
        let affected = conn.execute("DELETE FROM tag_galleries WHERE id = ?1", params![id])?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("标签图库 {id} 不存在")));
        }
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::migrate;
    use rusqlite::Connection;

    /// 建一个跑完全部迁移的空库。**这同时验证了 v10 的建表语句能被 SQLite 接受。**
    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn
    }

    /// 塞一条作品行。`page` 让同一个 pid 可以有多页 —— 多页各自算一条。
    fn add_artwork(conn: &Connection, pid: &str, page: i64) {
        conn.execute(
            "INSERT INTO artworks (pid, original_path, page, created_at) VALUES (?1, ?2, ?3, 0)",
            params![pid, format!("C:/a/{pid}_p{page}.jpg"), page],
        )
        .unwrap();
    }

    /// 塞一条元数据行。`restrict` 就是 `x_restrict`。
    fn add_meta(conn: &Connection, pid: &str, restrict: i64) {
        conn.execute(
            "INSERT INTO illust_meta (pid, title, x_restrict, fetched_at) VALUES (?1, ?2, ?3, 0)",
            params![pid, format!("作品 {pid}"), restrict],
        )
        .unwrap();
    }

    fn add_tag(conn: &Connection, pid: &str, tag: &str) {
        conn.execute(
            "INSERT INTO illust_tags (pid, tag) VALUES (?1, ?2)",
            params![pid, tag],
        )
        .unwrap();
    }

    #[test]
    fn rating_counts_splits_all_ages_nsfw_and_unknown() {
        let conn = db();
        add_artwork(&conn, "1", 0);
        add_meta(&conn, "1", 0); // 全年龄
        add_artwork(&conn, "2", 0);
        add_artwork(&conn, "2", 1); // 多页：两行都算
        add_meta(&conn, "2", 1); // R-18
        add_artwork(&conn, "3", 0); // 没抓过元数据

        let counts = rating_counts(&conn).unwrap();
        assert_eq!(counts.all_ages, 1, "全年龄 1 条");
        assert_eq!(counts.nsfw, 2, "R-18 作品的每一页都算一条");
        assert_eq!(counts.unknown, 1, "没元数据的那条算未知，不能落进全年龄");
        assert_eq!(
            counts.all_ages + counts.nsfw + counts.unknown,
            4,
            "三类必须盖住全部作品 —— 掉出去的作品在界面上就是「凭空消失」"
        );
    }

    #[test]
    fn gallery_count_uses_and_semantics_for_tags_and_splits_by_rating() {
        let conn = db();
        add_artwork(&conn, "10", 0);
        add_tag(&conn, "10", "鸣潮");
        add_tag(&conn, "10", "R-18");
        add_meta(&conn, "10", 1);

        add_artwork(&conn, "11", 0);
        add_tag(&conn, "11", "鸣潮");
        add_meta(&conn, "11", 0);

        add_artwork(&conn, "12", 0);
        add_tag(&conn, "12", "原神");
        add_meta(&conn, "12", 0);

        let t = |s: &str| s.to_string();
        assert_eq!(gallery_count(&conn, &[t("鸣潮")], RATING_ANY).unwrap(), 2);
        assert_eq!(
            gallery_count(&conn, &[t("鸣潮"), t("R-18")], RATING_ANY).unwrap(),
            1,
            "多标签是「同时包含」，不是任一"
        );
        assert_eq!(gallery_count(&conn, &[t("鸣潮")], RATING_ALL_AGES).unwrap(), 1);
        assert_eq!(gallery_count(&conn, &[t("鸣潮")], RATING_NSFW).unwrap(), 1);
        assert_eq!(gallery_count(&conn, &[], RATING_NSFW).unwrap(), 1, "只按分级");
        assert_eq!(gallery_count(&conn, &[], RATING_ANY).unwrap(), 3, "无条件=全部");
    }

    #[test]
    fn corrupt_tags_json_degrades_to_empty_instead_of_failing() {
        assert_eq!(parse_tags(r#"["a","b"]"#), vec!["a".to_string(), "b".to_string()]);
        assert!(
            parse_tags("这不是 JSON").is_empty(),
            "坏记录当空列表 —— 宁可少筛，也别让整个侧栏读不出来"
        );
    }
}

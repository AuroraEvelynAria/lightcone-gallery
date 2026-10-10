//! 从文件路径反推画师。
//!
//! pixez 这类下载器落盘时就已经按 `<画师名>_<画师ID>/<pid>_p<n>.<ext>` 分好目录了，
//! 所以画师信息**不需要联网**——它一直躺在路径里。实测本地 2653 条作品里
//! 2621 条（98.8%）能从路径解析出画师，1237 位。
//!
//! ## 为什么 uid 才是身份
//!
//! 同一 uid 在本地数据里出现过 **12 组不同名字**（画师改过名、或下载时把展会后缀
//! 写进了目录名），例如：
//!
//! ```text
//! 27848445 -> Coria / Coria@COMIC1☆24 / Coria@コミ1【E31b】
//! 11844841 -> Iras17514 / humugo / 爱画画的噗叽
//! ```
//!
//! 所以 `artists` 表以 uid 为主键，名字只是**显示标签**，而且允许同名不同 uid、
//! 同 uid 不同名。界面要能容纳这一点，不能假设「名字唯一」。

use std::path::Path;

use crate::db::models::ArtistSummary;
use crate::error::AppResult;

/// 从图片路径解析出画师 `(uid, name)`。
///
/// 取的是**直接父目录**的目录名，格式为 `<画师名>_<画师ID>`。
/// 返回 `None` 表示父目录不符合这个格式（例如图片直接躺在图库根目录下）。
///
/// 解析规则刻意用「贪婪匹配 + 锚定末尾数字」：
/// - `QuAn__6657532`     -> `("6657532", "QuAn_")`     名字本身以下划线结尾
/// - `Ciza__12345`       -> `("12345", "Ciza_")`
/// - `AkiZero1510_999`   -> `("999", "AkiZero1510")`   名字里含数字
/// - `0032_12345`        -> `("12345", "0032")`        名字全是数字
/// - `abc_123_456`       -> `("456", "abc_123")`       最后一个 `_数字` 才是 uid
///
/// 本地数据里画师名含下划线的有 47 个、含数字的有 71 个，上面这几条都真实存在，
/// 不能简化成「按第一个下划线切一刀」。
pub fn parse_artist(path: &str) -> Option<(String, String)> {
    let parent = Path::new(path).parent()?;
    let dir = parent.file_name()?.to_str()?;

    // 从末尾往前扫出连续数字段
    let digits_start = dir
        .char_indices()
        .rev()
        .take_while(|(_, ch)| ch.is_ascii_digit())
        .last()
        .map(|(index, _)| index)?;
    if digits_start == 0 {
        // 整个目录名都是数字，没有 `_` 分隔，不认为是 `名字_id`
        return None;
    }

    let (head, tail) = dir.split_at(digits_start);
    let name = head.strip_suffix('_')?;
    if name.is_empty() {
        return None;
    }

    Some((tail.to_string(), name.to_string()))
}

/// 把路径里解析到的画师登记进 `artists` 表。
///
/// **先到先得**（`DO NOTHING`）：同一个 uid 可能对应多个历史名字（见模块注释），
/// 这里保留最早遇到的那个。回填按 `id` 升序遍历，所以结果是确定的。
/// 换名字的代价只是显示上用的是旧名，不影响归类的正确性 —— 分组始终按 uid。
pub fn remember_artist(
    tx: &rusqlite::Transaction<'_>,
    artist: &Option<(String, String)>,
    now: i64,
) -> rusqlite::Result<()> {
    let Some((uid, name)) = artist else {
        return Ok(());
    };
    tx.execute(
        "INSERT INTO artists (uid, name, created_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(uid) DO NOTHING",
        rusqlite::params![uid, name, now],
    )?;
    Ok(())
}

/// 列出所有画师及其作品数，按总量倒序。
///
/// 两张表分别统计而不是 `UNION` 后一次分组：同一个文件可能同时被通用图库和作品库
/// 索引，两边各自计数更诚实 —— 界面要显示的是「在这套索引里有多少」。
pub fn list(conn: &rusqlite::Connection) -> AppResult<Vec<ArtistSummary>> {
    let mut stmt = conn.prepare(
        "SELECT a.uid,
                a.name,
                (SELECT COUNT(*) FROM images   WHERE artist_uid = a.uid) AS image_count,
                (SELECT COUNT(*) FROM artworks WHERE artist_uid = a.uid) AS artwork_count
         FROM artists a
         ORDER BY (image_count + artwork_count) DESC, a.name ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        // SQLite 只给 i64，而 usize 没实现 FromSql，这里显式转一次
        let images: i64 = row.get(2)?;
        let artworks: i64 = row.get(3)?;
        Ok(ArtistSummary {
            uid: row.get(0)?,
            name: row.get(1)?,
            images: images.max(0) as usize,
            artworks: artworks.max(0) as usize,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_plain_shape() {
        assert_eq!(
            parse_artist(r"D:\pixez pics\pixez\QuAn__6657532\150299063_p0.png"),
            Some(("6657532".to_string(), "QuAn_".to_string()))
        );
    }

    /// 名字以下划线结尾 —— 本地数据里有 47 个这样的画师。
    #[test]
    fn keeps_a_trailing_underscore_in_the_name() {
        assert_eq!(
            parse_artist("/root/Ciza__41583659/1_p0.jpg"),
            Some(("41583659".to_string(), "Ciza_".to_string()))
        );
        assert_eq!(
            parse_artist("/root/Eryuhe__43413952/1_p0.jpg"),
            Some(("43413952".to_string(), "Eryuhe_".to_string()))
        );
    }

    /// 名字里含数字 —— 本地数据里有 71 个，不能被误当成 uid。
    #[test]
    fn keeps_digits_that_belong_to_the_name() {
        assert_eq!(
            parse_artist("/root/AkiZero1510_12227191/1_p0.jpg"),
            Some(("12227191".to_string(), "AkiZero1510".to_string()))
        );
        assert_eq!(
            parse_artist("/root/0032_18679994/1_p0.jpg"),
            Some(("18679994".to_string(), "0032".to_string()))
        );
        assert_eq!(
            parse_artist("/root/1i1_93849478/1_p0.jpg"),
            Some(("93849478".to_string(), "1i1".to_string()))
        );
    }

    /// 名字里也带 `_数字` 时，只有最后一段是 uid。
    #[test]
    fn takes_the_last_numeric_segment_as_uid() {
        assert_eq!(
            parse_artist("/root/abc_123_456/1_p0.jpg"),
            Some(("456".to_string(), "abc_123".to_string()))
        );
    }

    /// 不符合格式的一律返回 None，调用方按「未知画师」处理。
    #[test]
    fn rejects_directories_without_an_artist_suffix() {
        // 图片直接躺在图库根目录下（本地有 64 张这样的）
        assert_eq!(parse_artist(r"D:\pixez pics\pixez\150299063_p0.png"), None);
        // 没有下划线
        assert_eq!(parse_artist("/root/12345/1_p0.jpg"), None);
        // 全是数字，没有名字部分
        assert_eq!(parse_artist("/root/_12345/1_p0.jpg"), None);
        // 末尾不是数字
        assert_eq!(parse_artist("/root/artist_name/1_p0.jpg"), None);
        // 没有父目录
        assert_eq!(parse_artist("1_p0.jpg"), None);
    }
}

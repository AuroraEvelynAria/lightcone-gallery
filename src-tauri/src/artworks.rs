//! 阶段一：Pixiv 作品维度的索引。
//!
//! 与 `scanner.rs` 的分工：
//! - `scanner.rs` 索引目录下所有能解码的图片，写进 `images` 表，供通用图库浏览（标签 / 相册 / 缩略图挂在它上面）
//! - 本模块只收录文件名符合 `{pid}_p{num}` 的文件，写进 `artworks` 表，是作品维度的索引
//!
//! 两个索引互不影响，因此新增作品维度不会动到已有图库的任何功能。

use std::path::Path;

use rusqlite::{Connection, params};
use walkdir::WalkDir;

use crate::db::models::ArtworkScanSummary;
use crate::db::now_unix;
use crate::error::{AppError, AppResult};
use crate::scanner::{is_supported_image, should_descend};

/// 从文件名解析 Pixiv 作品号与页码，格式为 `{pid}_p{num}.{ext}`。
///
/// 返回 `None` 表示命名不符合规范，调用方应跳过该文件而不是报错。
///
/// 容忍以下形态（Pixiv 下载器常追加后缀）：
/// - `123456_p0.jpg`           -> ("123456", 0)
/// - `123456_p12.png`          -> ("123456", 12)
/// - `123456_p0_master1200.jpg`-> ("123456", 0)
/// - `C:/pics/123456_p0.jpg`   -> ("123456", 0)   传完整路径也可以
pub fn parse_pid(filename: &str) -> Option<(String, i64)> {
    // 容忍传入完整路径，只取最后一段文件名
    let name = Path::new(filename).file_name()?.to_str()?;

    let (pid, rest) = name.split_once('_')?;
    if pid.is_empty() || !pid.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    // `_p` 之后取连续数字即可，多余后缀一律忽略
    let digits: String = rest
        .strip_prefix('p')?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        return None;
    }

    Some((pid.to_string(), digits.parse::<i64>().ok()?))
}

/// 遍历目录，把符合 `{pid}_p{num}` 命名的图片收进 `artworks` 表。
///
/// 去重由 `UNIQUE(pid, page)` 索引保证：重复文件不报错，只计入 `duplicates`。
/// 注意是「保留先入者」语义（`DO NOTHING`），即文件被移动后旧路径不会自动刷新。
pub fn scan_folder(conn: &mut Connection, root: &Path) -> AppResult<ArtworkScanSummary> {
    if !root.is_dir() {
        return Err(AppError::InvalidPath(format!(
            "目录不存在或不可访问: {}",
            root.display()
        )));
    }

    let mut discovered = 0usize;
    let mut matched: Vec<(String, i64, String)> = Vec::new();

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(should_descend)
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() || !is_supported_image(entry.path()) {
            continue;
        }
        discovered += 1;

        let Some(name) = entry.path().file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some((pid, page)) = parse_pid(name) else {
            continue;
        };
        matched.push((pid, page, entry.path().to_string_lossy().into_owned()));
    }

    let now = now_unix();
    let mut inserted = 0usize;
    let mut duplicates = 0usize;

    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO artworks (pid, file_path, page, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(pid, page) DO NOTHING",
        )?;
        for (pid, page, file_path) in &matched {
            // changes() 返回 0 即表示这一行被唯一索引挡掉了
            if stmt.execute(params![pid, file_path, page, now])? > 0 {
                inserted += 1;
            } else {
                duplicates += 1;
            }
        }
    }
    tx.commit()?;

    let matched_count = matched.len();
    Ok(ArtworkScanSummary {
        discovered,
        matched: matched_count,
        inserted,
        duplicates,
        skipped: discovered - matched_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    #[test]
    fn parse_pid_accepts_pixiv_naming() {
        assert_eq!(parse_pid("123456_p0.jpg"), Some(("123456".into(), 0)));
        assert_eq!(parse_pid("123456_p12.png"), Some(("123456".into(), 12)));
        // 下载器常追加后缀
        assert_eq!(parse_pid("123456_p0_master1200.jpg"), Some(("123456".into(), 0)));
        // 传完整路径也要能解析
        assert_eq!(parse_pid(r"C:\pics\123456_p3.webp"), Some(("123456".into(), 3)));
    }

    #[test]
    fn parse_pid_rejects_non_pixiv_naming() {
        assert_eq!(parse_pid("IMG_1234.jpg"), None, "pid 段必须是纯数字");
        assert_eq!(parse_pid("123456.jpg"), None, "缺少 _p{{num}} 段");
        assert_eq!(parse_pid("123456_p.jpg"), None, "页码不能为空");
        assert_eq!(parse_pid("_p0.jpg"), None, "pid 不能为空");
        assert_eq!(parse_pid("pid_p0.jpg"), None, "pid 不能含非数字");
    }

    fn make_root(name: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("lightcone-artworks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_image(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        image::RgbImage::new(4, 4).save(path).unwrap();
    }

    fn open_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        schema::migrate(&conn).unwrap();
        conn
    }

    /// 覆盖阶段一的核心链路：扫描 -> 解析 PID -> 入库 -> 多页不丢 -> 重复去重。
    #[test]
    fn scan_folder_indexes_all_pages_and_dedupes() {
        let root = make_root("scan");
        write_image(&root.join("123456_p0.jpg"));
        write_image(&root.join("123456_p1.jpg"));
        write_image(&root.join("123456_p2.jpg"));
        write_image(&root.join("789_p0.png"));
        // 不符合命名规范，应被跳过
        write_image(&root.join("IMG_1234.jpg"));
        std::fs::write(root.join("notes.txt"), b"not an image").unwrap();

        let mut conn = open_db();
        let summary = scan_folder(&mut conn, &root).unwrap();

        assert_eq!(summary.discovered, 5, "只统计图片文件，txt 不算");
        assert_eq!(summary.matched, 4);
        assert_eq!(summary.inserted, 4);
        assert_eq!(summary.duplicates, 0);
        assert_eq!(summary.skipped, 1, "IMG_1234.jpg 应被跳过");

        // 多页作品必须保留 3 条，不能塌缩成 1 条
        let pages: Vec<i64> = {
            let mut stmt = conn
                .prepare("SELECT page FROM artworks WHERE pid = '123456' ORDER BY page")
                .unwrap();
            let rows = stmt.query_map([], |row| row.get(0)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert_eq!(pages, vec![0, 1, 2]);

        // 再扫一次：全部命中唯一索引，一条都不新增
        let again = scan_folder(&mut conn, &root).unwrap();
        assert_eq!(again.inserted, 0);
        assert_eq!(again.duplicates, 4);

        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM artworks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, 4);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_folder_rejects_missing_directory() {
        let mut conn = open_db();
        let missing = std::env::temp_dir().join("lightcone-no-such-dir-artworks");
        assert!(scan_folder(&mut conn, &missing).is_err());
    }

    /// 扫描器写进库的 `file_path`，缩略图流水线必须能直接吃下去。
    ///
    /// 这是两个模块之间唯一的接缝：路径若被写成相对路径、或换了分隔符，
    /// 各自的单元测试都不会发现，只有把两段接起来才暴露。
    #[test]
    fn scanned_file_path_feeds_the_thumbnail_pipeline() {
        use crate::thumbnails::{self, Owner};

        let root = make_root("pipeline");
        // 用真实尺寸，确保走的是「解码 -> 缩放 -> 编码」而不是原样拷贝
        image::RgbImage::new(900, 1200)
            .save(root.join("654321_p0.jpg"))
            .unwrap();

        let mut conn = open_db();
        scan_folder(&mut conn, &root).unwrap();

        let (id, file_path): (i64, String) = conn
            .query_row(
                "SELECT id, file_path FROM artworks WHERE pid = '654321'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(
            Path::new(&file_path).is_file(),
            "库里存的路径应指向真实文件: {file_path}"
        );

        // 缓存目录建在扫描之后，不会反过来影响扫描结果
        let cache = root.join("cache");
        let thumb =
            thumbnails::ensure_thumbnail(&cache, Owner::Artwork, id, Path::new(&file_path), 320)
                .unwrap();

        let bytes = std::fs::read(&thumb).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WEBP", "作品缩略图必须是 WebP");

        let (width, height) = image::image_dimensions(&thumb).unwrap();
        assert!(
            width <= 320 && height <= 320 && height > width,
            "应等比缩放到 320 以内的竖图，实际 {width}x{height}"
        );
        assert!(
            thumb.to_string_lossy().contains("artworks"),
            "作品缩略图必须落在 artworks 子目录，否则会和图库的 images 撞车: {}",
            thumb.display()
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}

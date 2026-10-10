//! 阶段一：Pixiv 作品维度的索引。
//! 阶段二：扫描后批量生成有损 WebP 缩略图，并把路径回填数据库。
//!
//! 与 `scanner.rs` 的分工：
//! - `scanner.rs` 索引目录下所有能解码的图片，写进 `images` 表，供通用图库浏览（标签 / 相册 / 缩略图挂在它上面）
//! - 本模块只收录文件名符合 `{pid}_p{num}` 的文件，写进 `artworks` 表，是作品维度的索引
//!
//! 两个索引互不影响，因此新增作品维度不会动到已有图库的任何功能。

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};
use walkdir::WalkDir;

use crate::db::models::{ArtworkScanSummary, ThumbnailBatch};
use crate::db::now_unix;
use crate::error::{AppError, AppResult};
use crate::scanner::{is_supported_image, should_descend};
use crate::thumbnails::{self, Owner};

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

/// 遍历目录，把符合 `{pid}_p{num}` 命名的图片收进 `artworks` 表，并登记扫描根目录。
///
/// 去重由 `UNIQUE(pid, page)` 索引保证：重复文件不报错，只计入 `duplicates`。
/// 注意是「保留先入者」语义（`DO NOTHING`），即文件被移动后旧路径不会自动刷新。
///
/// 本函数**不生成缩略图**（那是 CPU 密集的，单独一个 pass），
/// 返回的 summary 里三个 `thumbnails_*` 字段都是 0，由 `generate_thumbnails` 回填。
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
        // thumbnail_path 先留空，等 generate_thumbnails 回填
        let mut stmt = tx.prepare(
            "INSERT INTO artworks (pid, original_path, page, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(pid, page) DO NOTHING",
        )?;
        for (pid, page, original_path) in &matched {
            // changes() 返回 0 即表示这一行被唯一索引挡掉了
            if stmt.execute(params![pid, original_path, page, now])? > 0 {
                inserted += 1;
            } else {
                duplicates += 1;
            }
        }

        // 根目录落盘。原图要经 asset:// 显示，而该协议的作用域只存在于内存中
        // （见 lib.rs 的启动流程），不记下来就没法在重启后重新授权。
        tx.execute(
            "INSERT INTO artwork_roots (path, created_at) VALUES (?1, ?2)
             ON CONFLICT(path) DO NOTHING",
            params![root.to_string_lossy(), now],
        )?;
    }
    tx.commit()?;

    let matched_count = matched.len();
    Ok(ArtworkScanSummary {
        discovered,
        matched: matched_count,
        inserted,
        duplicates,
        skipped: discovered - matched_count,
        thumbnails_generated: 0,
        thumbnails_skipped: 0,
        thumbnails_failed: 0,
    })
}

/// 为库里缺缩略图（或缓存已过期）的作品生成缩略图，并把路径回填 `thumbnail_path`。
///
/// 这个 pass 是幂等的：缓存仍然新鲜的一律跳过，所以重复扫描同一目录几乎不花时间。
/// 缓存文件被手动删掉（例如侧栏的「清理」）也会被这里重新补齐。
pub fn generate_thumbnails(
    conn: &mut Connection,
    cache_dir: &Path,
    size: u32,
    on_progress: &(dyn Fn(usize, usize) + Sync),
) -> AppResult<ThumbnailBatch> {
    let rows: Vec<(i64, String, Option<String>)> = {
        let mut stmt = conn.prepare("SELECT id, original_path, thumbnail_path FROM artworks")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut pending: Vec<(i64, PathBuf)> = Vec::new();
    let mut skipped = 0usize;

    for (id, original, stored) in rows {
        let source = PathBuf::from(&original);
        // 路径列有值但文件已经不在（被清理掉、或盘上被删）也要重算，
        // 否则前端会一直拿着一个 404 的地址
        let fresh = stored.as_deref().is_some_and(|_| {
            thumbnails::is_cached(cache_dir, Owner::Artwork, id, &source, size)
        });

        if fresh {
            skipped += 1;
        } else {
            pending.push((id, source));
        }
    }

    let wanted = pending.len();
    let rendered = thumbnails::ensure_parallel(cache_dir, Owner::Artwork, &pending, size, on_progress);

    // 缩略图已经落盘，路径串行写回 —— 线程池里不碰数据库连接
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare("UPDATE artworks SET thumbnail_path = ?1 WHERE id = ?2")?;
        for (id, path) in &rendered {
            stmt.execute(params![path.to_string_lossy(), id])?;
        }
    }
    tx.commit()?;

    Ok(ThumbnailBatch {
        generated: rendered.len(),
        skipped,
        failed: wanted - rendered.len(),
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

        // 扫描根目录必须落盘：asset:// 的作用域不持久化，重启后要靠它重新授权
        let roots: Vec<String> = {
            let mut stmt = conn.prepare("SELECT path FROM artwork_roots").unwrap();
            let rows = stmt.query_map([], |row| row.get(0)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert_eq!(roots, vec![root.to_string_lossy().to_string()]);
        assert_eq!(again.inserted + again.duplicates, 4, "重复扫描不得重复登记根目录");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_folder_rejects_missing_directory() {
        let mut conn = open_db();
        let missing = std::env::temp_dir().join("lightcone-no-such-dir-artworks");
        assert!(scan_folder(&mut conn, &missing).is_err());
    }

    /// 扫描器写进库的 `original_path`，缩略图流水线必须能直接吃下去。
    ///
    /// 这是两个模块之间唯一的接缝：路径若被写成相对路径、或换了分隔符，
    /// 各自的单元测试都不会发现，只有把两段接起来才暴露。
    #[test]
    fn scanned_original_path_feeds_the_thumbnail_pipeline() {
        use crate::thumbnails::{self, Owner};

        let root = make_root("pipeline");
        // 用真实尺寸，确保走的是「解码 -> 缩放 -> 编码」而不是原样拷贝
        image::RgbImage::new(900, 1200)
            .save(root.join("654321_p0.jpg"))
            .unwrap();

        let mut conn = open_db();
        scan_folder(&mut conn, &root).unwrap();

        let (id, original): (i64, String) = conn
            .query_row(
                "SELECT id, original_path FROM artworks WHERE pid = '654321'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(
            Path::new(&original).is_file(),
            "库里存的路径应指向真实文件: {original}"
        );

        // 缓存目录建在扫描之后，不会反过来影响扫描结果
        let cache = root.join("cache");
        let thumb = thumbnails::ensure_thumbnail(
            &cache,
            Owner::Artwork,
            id,
            Path::new(&original),
            thumbnails::ARTWORK_SIZE,
        )
        .unwrap();

        let bytes = std::fs::read(&thumb).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WEBP", "作品缩略图必须是 WebP");

        let (width, height) = image::image_dimensions(&thumb).unwrap();
        assert!(
            width <= 400 && height <= 400 && height > width,
            "应等比缩放到 400 以内的竖图，实际 {width}x{height}"
        );
        assert!(
            thumb.to_string_lossy().contains("artworks"),
            "作品缩略图必须落在 artworks 子目录，否则会和图库的 images 撞车: {}",
            thumb.display()
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 扫描 -> 批量生成缩略图 -> 路径入库；再跑一次必须全部跳过；
    /// 缓存文件被删掉后必须能补回来（侧栏「清理缓存」后的自愈路径）。
    #[test]
    fn generate_thumbnails_fills_paths_then_skips_fresh() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let root = make_root("thumbs");
        write_image(&root.join("111_p0.jpg"));
        write_image(&root.join("111_p1.jpg"));
        write_image(&root.join("222_p0.png"));

        let cache = make_root("thumbs-cache");
        let mut conn = open_db();
        scan_folder(&mut conn, &root).unwrap();

        // 扫描本身不生成缩略图，路径列应先留空
        let empty: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artworks WHERE thumbnail_path IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(empty, 3);

        let calls = AtomicUsize::new(0);
        let count = |_: usize, _: usize| {
            calls.fetch_add(1, Ordering::Relaxed);
        };

        let first = generate_thumbnails(&mut conn, &cache, 400, &count).unwrap();
        assert_eq!(first.generated, 3);
        assert_eq!(first.skipped, 0);
        assert_eq!(first.failed, 0);
        assert_eq!(calls.load(Ordering::Relaxed), 3, "每张都应上报一次进度");

        let paths: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT thumbnail_path FROM artworks ORDER BY id")
                .unwrap();
            let rows = stmt.query_map([], |row| row.get(0)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert_eq!(paths.len(), 3);
        for path in &paths {
            let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path} 读不到: {e}"));
            assert_eq!(&bytes[8..12], b"WEBP", "{path} 不是 WebP");
            assert!(path.contains("artworks"), "应落在 artworks 子目录: {path}");
        }

        // 再跑一次：全部命中缓存，一张都不重算
        let second = generate_thumbnails(&mut conn, &cache, 400, &count).unwrap();
        assert_eq!(second.generated, 0, "缓存新鲜时不得重复生成");
        assert_eq!(second.skipped, 3);

        // 删掉一个缓存文件 -> 下次必须补齐
        std::fs::remove_file(&paths[0]).unwrap();
        let third = generate_thumbnails(&mut conn, &cache, 400, &count).unwrap();
        assert_eq!(third.generated, 1, "缓存文件缺失应被补齐");
        assert_eq!(third.skipped, 2);

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&cache);
    }

    /// 源文件缺失时不能把整个 pass 拖垮，只计入 failed。
    #[test]
    fn generate_thumbnails_counts_missing_sources_as_failed() {
        let root = make_root("thumbs-missing");
        write_image(&root.join("333_p0.jpg"));
        write_image(&root.join("333_p1.jpg"));

        let cache = make_root("thumbs-missing-cache");
        let mut conn = open_db();
        scan_folder(&mut conn, &root).unwrap();

        // 索引完就把其中一张原图删掉，模拟「盘上文件已消失」
        std::fs::remove_file(root.join("333_p1.jpg")).unwrap();

        let batch = generate_thumbnails(&mut conn, &cache, 400, &|_, _| {}).unwrap();
        assert_eq!(batch.generated, 1);
        assert_eq!(batch.failed, 1, "缺失的源文件应计入 failed 而不是让 pass 失败");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&cache);
    }
}

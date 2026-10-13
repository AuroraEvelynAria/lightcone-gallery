//! 阶段一：Pixiv 作品维度的索引。
//! 阶段二：扫描后批量生成有损 WebP 缩略图，并把路径回填数据库。
//!
//! 与 `scanner.rs` 的分工：
//! - `scanner.rs` 索引目录下所有能解码的图片，写进 `images` 表，供通用图库浏览（标签 / 相册 / 缩略图挂在它上面）
//! - 本模块只收录文件名符合 `{pid}_p{num}` 的文件，写进 `artworks` 表，是作品维度的索引
//!
//! 两个索引互不影响，因此新增作品维度不会动到已有图库的任何功能。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use walkdir::WalkDir;

use crate::db::models::{ArtworkScanSummary, ThumbnailBatch};
use crate::db::now_unix;
use crate::error::{AppError, AppResult};
use crate::scanner::{is_supported_image, should_descend};
use crate::thumbnails::{self, Owner};

/// 作品列表的排序。
///
/// **`list_artworks` 与 `list_aspects` 必须用同一句**：瀑布流是拿「第 i 张的比例」
/// 给「第 i 张作品」排版的，两边顺序一旦漂移，表现就是「格子高度和图对不上」——
/// 不报错、不崩溃，只是每张卡片的框和内容错位。所以它只能有一个出处。
pub const ARTWORK_ORDER: &str = "ORDER BY CAST(pid AS INTEGER) DESC, page ASC, id ASC";

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
    let mut matched: Vec<(String, i64, String, Option<(String, String)>)> = Vec::new();

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
        let path = entry.path().to_string_lossy().into_owned();
        // 画师从路径反推，不联网（见 crate::artists）
        let artist = crate::artists::parse_artist(&path);
        matched.push((pid, page, path, artist));
    }

    let now = now_unix();
    let mut inserted = 0usize;
    let mut duplicates = 0usize;
    let mut ignored = 0usize;

    // 忽略名单：之前被从作品库移除过的 `(pid, page)` 不再收回来。
    // 没有这一步，「移除」就只是删行，重扫同一个目录它们会原封不动地回来。
    let skipped_pairs: HashSet<(String, i64)> = {
        let mut stmt = conn.prepare("SELECT pid, page FROM ignored_artworks")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<HashSet<_>>>()?
    };

    let tx = conn.transaction()?;
    {
        // thumbnail_path 先留空，等 generate_thumbnails 回填
        let mut stmt = tx.prepare(
            "INSERT INTO artworks (pid, original_path, page, created_at, artist_uid)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(pid, page) DO NOTHING",
        )?;
        for (pid, page, original_path, artist) in &matched {
            if skipped_pairs.contains(&(pid.clone(), *page)) {
                ignored += 1;
                continue;
            }
            crate::artists::remember_artist(&tx, artist, now)?;
            // changes() 返回 0 即表示这一行被唯一索引挡掉了
            let artist_uid = artist.as_ref().map(|(uid, _)| uid.as_str());
            if stmt.execute(params![pid, original_path, page, now, artist_uid])? > 0 {
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
        ignored,
        thumbnails_generated: 0,
        thumbnails_skipped: 0,
        thumbnails_failed: 0,
    })
}

/// Android：MediaStore 桥回传的一张图。Kotlin 侧已按 `{pid}_p{n}.{ext}`
/// 正则过滤并解析好 pid / page，Rust 侧不再重复解析。
///
/// `path` 是真实文件路径（`relative_path + display_name`）。Android 11+ 的
/// FUSE 会放行持有 `READ_MEDIA_IMAGES` 的应用按路径读媒体文件，缩略图管线
/// 照桌面一样按路径读就行；个别设备策略不一致时，Kotlin 桥的 `copyToCache`
/// 是兜底（拷进应用 cache 再走老管线）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
// 只在 Android 分支（android_scan_media_store）被消费
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub struct MediaImageRow {
    /// MediaStore 的 `_id`。走 `copyToCache` 兜底时用它拼 content URI。
    #[allow(dead_code)]
    pub id: i64,
    #[allow(dead_code)]
    pub name: String,
    pub pid: String,
    pub page: i64,
    pub path: String,
    #[allow(dead_code)]
    pub size: i64,
    #[allow(dead_code)]
    pub modified: i64,
    #[allow(dead_code)]
    pub width: i64,
    #[allow(dead_code)]
    pub height: i64,
}

/// Android 入库：与 [`scan_folder`] 完全同一套语义——忽略名单、`UNIQUE(pid, page)`
/// 去重计数、画师从路径反推、根目录落盘——只是行来自 MediaStore 桥而不是 walkdir。
///
/// 根目录按每行的父目录去重登记（pixez 的目录结构是 `<画师名>_<uid>/<pid>_p<n>`，
/// 路径反推画师的规则照常生效）；asset:// 的**运行时**授权在命令层做。
/// `discovered` / `skipped` 与桌面语义对齐：Kotlin 侧已按命名过滤，进来的行
/// 全部视为「命名匹配」。
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn scan_media_rows(
    conn: &mut Connection,
    rows: &[MediaImageRow],
) -> AppResult<ArtworkScanSummary> {
    let now = now_unix();
    let mut roots: HashSet<String> = HashSet::new();
    let mut matched: Vec<(String, i64, String, Option<(String, String)>)> = Vec::new();

    for row in rows {
        matched.push((
            row.pid.clone(),
            row.page,
            row.path.clone(),
            crate::artists::parse_artist(&row.path),
        ));
        if let Some(parent) = Path::new(&row.path).parent() {
            roots.insert(parent.to_string_lossy().into_owned());
        }
    }

    let mut inserted = 0usize;
    let mut duplicates = 0usize;
    let mut ignored = 0usize;

    // 清掉首版桥带进来的坏路径行（相对路径 + 双斜杠——MediaStore 的
    // RELATIVE_PATH 自带尾斜杠，首扫没拼存储根前缀）。它们不删的话，
    // 修好的路径会被 ON CONFLICT DO NOTHING 挡在外面，缩略图永远修不回来。
    // Android 绝对路径必以 / 开头，此清理只影响桥的存量，不碰桌面行。
    conn.execute("DELETE FROM artworks WHERE original_path NOT LIKE '/%'", [])?;

    let skipped_pairs: HashSet<(String, i64)> = {
        let mut stmt = conn.prepare("SELECT pid, page FROM ignored_artworks")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<HashSet<_>>>()?
    };

    let tx = conn.transaction()?;
    {
        // 与 scan_folder 同一条 INSERT：thumbnail_path 先留空，等 generate_thumbnails 回填
        let mut stmt = tx.prepare(
            "INSERT INTO artworks (pid, original_path, page, created_at, artist_uid)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(pid, page) DO NOTHING",
        )?;
        for (pid, page, original_path, artist) in &matched {
            if skipped_pairs.contains(&(pid.clone(), *page)) {
                ignored += 1;
                continue;
            }
            crate::artists::remember_artist(&tx, artist, now)?;
            let artist_uid = artist.as_ref().map(|(uid, _)| uid.as_str());
            if stmt.execute(params![pid, original_path, page, now, artist_uid])? > 0 {
                inserted += 1;
            } else {
                duplicates += 1;
            }
        }

        for root in &roots {
            tx.execute(
                "INSERT INTO artwork_roots (path, created_at) VALUES (?1, ?2)
                 ON CONFLICT(path) DO NOTHING",
                params![root, now],
            )?;
        }
    }
    tx.commit()?;

    let matched_count = matched.len();
    Ok(ArtworkScanSummary {
        discovered: rows.len(),
        matched: matched_count,
        inserted,
        duplicates,
        skipped: rows.len() - matched_count,
        ignored,
        thumbnails_generated: 0,
        thumbnails_skipped: 0,
        thumbnails_failed: 0,
    })
}

/// 把作品移出作品库：删掉数据库记录与对应的缩略图缓存文件，返回实际删掉的行数。
///
/// **绝不碰原图** —— 这是「移出索引」而不是删除文件，应用对图库目录始终只读。
/// 这条承诺由 `remove_drops_rows_and_thumbnails_but_never_the_original` 守着。
///
/// 顺带把 `(pid, page)` 记进忽略名单，重扫同一目录不会再收回来。
/// 不这么做的话「移除」只是删行，用户会看到它们原封不动地回来。
/// 记的是 `pid` 而不是 `id`：id 是自增的，重扫会换一个，只有 `(pid, page)` 稳定。
/// 想反悔就用 `clear_ignored` 清空名单，之后重扫会把它们再收回来。
///
/// 缩略图缓存顺手清掉：它按 `artworks.id` 命名，行删了以后重扫会拿到新的自增 id，
/// 旧文件永远不会再被读到，留着就是孤儿。
pub fn remove(conn: &mut Connection, cache_dir: &Path, ids: &[i64]) -> AppResult<usize> {
    if ids.is_empty() {
        return Ok(0);
    }

    let now = now_unix();
    let tx = conn.transaction()?;
    let mut removed = 0usize;
    {
        // 先问出身份再删，所以查和删要成对出现
        let mut select = tx.prepare("SELECT pid, page FROM artworks WHERE id = ?1")?;
        let mut delete = tx.prepare("DELETE FROM artworks WHERE id = ?1")?;
        let mut ignore = tx.prepare(
            "INSERT INTO ignored_artworks (pid, page, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(pid, page) DO NOTHING",
        )?;

        for id in ids {
            let identity: Option<(String, i64)> = select
                .query_row([id], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()?;

            // 传进来的 id 可能有一部分已经不存在了，按实际影响行数计
            if delete.execute([id])? == 0 {
                continue;
            }
            removed += 1;

            if let Some((pid, page)) = identity {
                ignore.execute(params![pid, page, now])?;
            }
        }
    }
    tx.commit()?;

    for id in ids {
        let thumb = thumbnails::thumbnail_path(
            cache_dir,
            Owner::Artwork,
            *id,
            thumbnails::ARTWORK_SIZE,
        );
        // 删不掉也无所谓（可能本来就没生成过），不因此让整个操作失败
        let _ = std::fs::remove_file(thumb);
    }

    Ok(removed)
}

/// 忽略名单里有多少条 `(pid, page)`。
pub fn count_ignored(conn: &Connection) -> AppResult<usize> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM ignored_artworks", [], |row| row.get(0))?;
    Ok(count as usize)
}

/// 清空忽略名单，返回清掉的条数。之后重新扫描目录会把它们再收回来。
pub fn clear_ignored(conn: &Connection) -> AppResult<usize> {
    Ok(conn.execute("DELETE FROM ignored_artworks", [])?)
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
            width <= thumbnails::ARTWORK_SIZE
                && height <= thumbnails::ARTWORK_SIZE
                && height > width,
            "应等比缩放到 {} 以内的竖图，实际 {width}x{height}",
            thumbnails::ARTWORK_SIZE
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

        let first = generate_thumbnails(&mut conn, &cache, thumbnails::ARTWORK_SIZE, &count).unwrap();
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
        let second = generate_thumbnails(&mut conn, &cache, thumbnails::ARTWORK_SIZE, &count).unwrap();
        assert_eq!(second.generated, 0, "缓存新鲜时不得重复生成");
        assert_eq!(second.skipped, 3);

        // 删掉一个缓存文件 -> 下次必须补齐
        std::fs::remove_file(&paths[0]).unwrap();
        let third = generate_thumbnails(&mut conn, &cache, thumbnails::ARTWORK_SIZE, &count).unwrap();
        assert_eq!(third.generated, 1, "缓存文件缺失应被补齐");
        assert_eq!(third.skipped, 2);

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&cache);
    }

    /// 移除只删记录与缩略图缓存，**绝不能碰原图** —— 这是这个功能的核心承诺。
    #[test]
    fn remove_drops_rows_and_thumbnails_but_never_the_original() {
        let root = make_root("remove");
        write_image(&root.join("444_p0.jpg"));
        write_image(&root.join("444_p1.jpg"));
        write_image(&root.join("555_p0.jpg"));

        let cache = make_root("remove-cache");
        let mut conn = open_db();
        scan_folder(&mut conn, &root).unwrap();
        generate_thumbnails(&mut conn, &cache, thumbnails::ARTWORK_SIZE, &|_, _| {}).unwrap();

        let rows: Vec<(i64, String, String)> = {
            let mut stmt = conn
                .prepare("SELECT id, original_path, thumbnail_path FROM artworks ORDER BY id")
                .unwrap();
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert_eq!(rows.len(), 3);
        for (_, _, thumb) in &rows {
            assert!(Path::new(thumb).is_file(), "前置条件：缩略图应已生成 {thumb}");
        }

        let (target_id, target_original, target_thumb) = rows[0].clone();
        assert_eq!(remove(&mut conn, &cache, &[target_id]).unwrap(), 1);

        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM artworks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 2, "记录应被删掉");
        assert!(!Path::new(&target_thumb).exists(), "缩略图缓存应被清掉");
        assert!(
            Path::new(&target_original).is_file(),
            "移除绝不能删除原图: {target_original}"
        );

        // 不存在的 id 与空列表都不该报错
        assert_eq!(remove(&mut conn, &cache, &[999_999]).unwrap(), 0);
        assert_eq!(remove(&mut conn, &cache, &[]).unwrap(), 0);

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&cache);
    }

    /// 移除过的作品不该在重扫时回来 —— 否则「移除」在用户眼里等于没做。
    #[test]
    fn removed_artworks_stay_out_of_the_next_scan() {
        let root = make_root("ignore-rescan");
        write_image(&root.join("666_p0.jpg"));
        write_image(&root.join("666_p1.jpg"));
        write_image(&root.join("777_p0.jpg"));

        let cache = make_root("ignore-cache");
        let mut conn = open_db();
        scan_folder(&mut conn, &root).unwrap();

        let target: i64 = conn
            .query_row(
                "SELECT id FROM artworks WHERE pid = '666' AND page = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remove(&mut conn, &cache, &[target]).unwrap(), 1);
        assert_eq!(count_ignored(&conn).unwrap(), 1, "移除应写入忽略名单");

        // 重扫同一目录：666_p1 应当被忽略名单挡住
        let summary = scan_folder(&mut conn, &root).unwrap();
        assert_eq!(summary.ignored, 1, "被移除过的那一页应计入 ignored");
        assert_eq!(summary.inserted, 0, "不该有任何新行");
        let back: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artworks WHERE pid = '666' AND page = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(back, 0, "移除过的作品不该被重扫收回来");

        // 清空名单之后应当能重新收录
        assert_eq!(clear_ignored(&conn).unwrap(), 1);
        assert_eq!(count_ignored(&conn).unwrap(), 0);
        let summary = scan_folder(&mut conn, &root).unwrap();
        assert_eq!(summary.inserted, 1, "清空忽略名单后应当重新收录");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&cache);
    }

    /// 扫描时就把画师解析好写进行里，不必等回填。
    #[test]
    fn scan_records_the_artist_parsed_from_the_directory() {
        let root = make_root("scan-artist");
        let artist_dir = root.join("QuAn__6657532");
        std::fs::create_dir_all(&artist_dir).unwrap();
        write_image(&artist_dir.join("123456_p0.jpg"));
        // 直接躺在根目录下的那张解析不出画师
        write_image(&root.join("654321_p0.jpg"));

        let mut conn = open_db();
        scan_folder(&mut conn, &root).unwrap();

        let uid: Option<String> = conn
            .query_row(
                "SELECT artist_uid FROM artworks WHERE pid = '123456'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(uid.as_deref(), Some("6657532"));

        let name: String = conn
            .query_row("SELECT name FROM artists WHERE uid = '6657532'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(name, "QuAn_", "名字要保住末尾那个下划线");

        let orphan: Option<String> = conn
            .query_row(
                "SELECT artist_uid FROM artworks WHERE pid = '654321'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(orphan, None);

        let _ = std::fs::remove_dir_all(&root);
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

        let batch =
            generate_thumbnails(&mut conn, &cache, thumbnails::ARTWORK_SIZE, &|_, _| {}).unwrap();
        assert_eq!(batch.generated, 1);
        assert_eq!(batch.failed, 1, "缺失的源文件应计入 failed 而不是让 pass 失败");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&cache);
    }

    /// 瀑布流的比例表：顺序必须与列表一致，没量过的计入 pending，量过的直接用。
    ///
    /// 顺序错了不会报错、不会崩，只会让**每张卡片的框和内容错位** ——
    /// 这是这条链路上唯一一种「静默出错」，所以专门钉一条。
    #[test]
    fn scan_aspects_follows_list_order_and_splits_pending() {
        let root = make_root("aspects");
        for name in ["300_p0.jpg", "100_p0.jpg", "200_p0.jpg"] {
            write_image(&root.join(name));
        }

        let mut conn = open_db();
        scan_folder(&mut conn, &root).unwrap();

        // 只有 100 那条已经登记过宽高（竖图 600x1200 → 比例 2.0）
        conn.execute(
            "UPDATE artworks SET width = 600, height = 1200 WHERE pid = '100'",
            [],
        )
        .unwrap();

        let scan = scan_aspects(&conn, "", &[]).unwrap();
        let ids: Vec<i64> = scan.rows.iter().map(|row| row.id).collect();

        // 与列表同序：pid 数值倒序
        let listed: Vec<i64> = {
            let mut stmt = conn
                .prepare(&format!("SELECT id FROM artworks {ARTWORK_ORDER}"))
                .unwrap();
            let rows = stmt.query_map([], |row| row.get(0)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert_eq!(ids, listed, "比例表的顺序必须与作品列表一字不差");

        // 按 pid 取，不按下标 —— 顺序本身在上面已经断言过了，这里要验的是取值
        let aspect_of = |pid: &str| -> f64 {
            let id: i64 = conn
                .query_row("SELECT id FROM artworks WHERE pid = ?1", [pid], |row| row.get(0))
                .unwrap();
            scan.rows.iter().find(|row| row.id == id).unwrap().aspect()
        };

        assert_eq!(aspect_of("100"), 2.0, "登记过的那条应直接用库里的宽高");
        assert_eq!(scan.pending.len(), 2, "另外两条没量过，应计入 pending");
        assert_eq!(
            aspect_of("300"),
            FALLBACK_ASPECT,
            "还没量到的先用兜底值，不能是 0 或 NaN（那会让整列高成 0）"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 极端长图 / 宽图会被夹到 `[MIN_ASPECT, MAX_ASPECT]`，不让一格吃掉整屏。
    #[test]
    fn aspect_is_clamped_for_extreme_shapes() {
        let root = make_root("clamp");
        write_image(&root.join("1_p0.jpg"));
        write_image(&root.join("2_p0.jpg"));
        write_image(&root.join("3_p0.jpg"));

        let mut conn = open_db();
        scan_folder(&mut conn, &root).unwrap();
        conn.execute("UPDATE artworks SET width = 1000, height = 5000 WHERE pid = '1'", [])
            .unwrap(); // 1:5 的极端长图
        conn.execute("UPDATE artworks SET width = 5000, height = 1000 WHERE pid = '2'", [])
            .unwrap(); // 5:1 的极端宽图
        conn.execute("UPDATE artworks SET width = 0, height = 0 WHERE pid = '3'", [])
            .unwrap(); // 脏数据不能让宽高比变成 NaN

        let scan = scan_aspects(&conn, "", &[]).unwrap();
        let by_pid = |pid: &str| -> f64 {
            let id: i64 = conn
                .query_row("SELECT id FROM artworks WHERE pid = ?1", [pid], |row| row.get(0))
                .unwrap();
            scan.rows.iter().find(|row| row.id == id).unwrap().aspect()
        };

        assert_eq!(by_pid("1"), MAX_ASPECT, "1:5 应夹到上限");
        assert_eq!(by_pid("2"), MIN_ASPECT, "5:1 应夹到下限");
        assert!(by_pid("3").is_finite() && by_pid("3") > 0.0, "0 宽高的脏数据要走兜底值");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 量到尺寸之后写回库，再读一次就应该是精确值、且不再有 pending。
    #[test]
    fn store_dimensions_clears_the_backlog() {
        let root = make_root("store-dims");
        write_image(&root.join("9_p0.jpg"));

        let mut conn = open_db();
        scan_folder(&mut conn, &root).unwrap();

        let scan = scan_aspects(&conn, "", &[]).unwrap();
        assert_eq!(scan.pending.len(), 1);
        let (_, id, _) = scan.pending[0];

        assert_eq!(store_dimensions(&mut conn, &[(id, 1200, 800)]).unwrap(), 1);

        let after = scan_aspects(&conn, "", &[]).unwrap();
        assert!(after.pending.is_empty(), "写回之后不该再有待量的行");
        assert!((after.rows[0].aspect() - 800.0 / 1200.0).abs() < 1e-9);

        let _ = std::fs::remove_dir_all(&root);
    }
}


/// 瀑布流排版的兜底高宽比（高 / 宽）。
///
/// 只在这张图的宽高既没入库、又读不出文件头时用（原图被删、缩略图也没生成出来）。
/// 取本机 4430 张实测的**中位值 1.38** 向上取整：偏大偏小都会让那一列松紧失真，
/// 而中位意味着「猜错的代价最小」。
pub const FALLBACK_ASPECT: f64 = 1.4;

/// 极端长图 / 宽图的夹取范围（高 / 宽），与前端 `masonryGeometry` 的取值一致。
///
/// 超出范围的卡片会在框内被 `object-cover` 裁掉一点。本机实测夹到 `[0.5, 2.5]`
/// 只有 **2.6%** 的作品需要裁 —— 而正方网格是 100% 全裁。
const MIN_ASPECT: f64 = 0.5;
const MAX_ASPECT: f64 = 2.5;

/// 一行作品在排版时需要的东西：身份、探针路径（还没量过时用）、已有的尺寸。
#[derive(Debug, Clone)]
pub struct AspectRow {
    pub id: i64,
    probe: PathBuf,
    /// 原图像素宽高。`None` 表示还没量过，`aspect()` 会用兜底值。
    pub size: Option<(i64, i64)>,
}

impl AspectRow {
    /// 把刚量到的像素尺寸填进来（写库之外，调用方手里这份也要同步）。
    pub fn set_size(&mut self, width: u32, height: u32) {
        self.size = Some((width as i64, height as i64));
    }

    pub fn aspect(&self) -> f64 {
        match self.size {
            Some((width, height)) if width > 0 && height > 0 => {
                (height as f64 / width as f64).clamp(MIN_ASPECT, MAX_ASPECT)
            }
            _ => FALLBACK_ASPECT,
        }
    }
}

/// `scan_aspects` 的产物：比例表 + 还需要补量的那几条。
#[derive(Debug, Default)]
pub struct AspectScan {
    pub rows: Vec<AspectRow>,
    /// `(在 rows 里的下标, id, 探针路径)`。下标是给调用方回填后原地更新用的
    /// —— 拿 id 反查在 4430 条上就是 O(n²)。
    pub pending: Vec<(usize, i64, PathBuf)>,
}

/// 读出当前筛选下**每一张**作品的尺寸行，顺序与 `list_artworks` 一字不差。
///
/// 瀑布流要一次拿到全部比例才算得出滚动条长度（总高 = 最高那一列有多高），
/// 所以这里刻意不分页 —— 只送 id 与比例时 4430 条约 70KB，一次取完最省事。
///
/// **这个函数不碰文件系统**：量文件头是调用方在数据库锁之外做的事。一次回填要
/// 读几千个文件头，把锁一起攥住会让同期的列表查询白等好几秒。
pub fn scan_aspects(
    conn: &Connection,
    filter: &str,
    binds: &[rusqlite::types::Value],
) -> AppResult<AspectScan> {
    let mut stmt = conn.prepare(&format!(
        "SELECT id, thumbnail_path, original_path, width, height FROM artworks
         WHERE 1 = 1{filter} {ARTWORK_ORDER}"
    ))?;

    let rows = stmt
        .query_map(rusqlite::params_from_iter(binds.iter()), |row| {
            let thumbnail: Option<String> = row.get(1)?;
            let original: String = row.get(2)?;
            Ok(AspectRow {
                id: row.get(0)?,
                probe: pick_probe(thumbnail.as_deref(), &original),
                size: match (row.get::<_, Option<i64>>(3)?, row.get::<_, Option<i64>>(4)?) {
                    (Some(width), Some(height)) => Some((width, height)),
                    _ => None,
                },
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let pending = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.size.is_none())
        .map(|(index, row)| (index, row.id, row.probe.clone()))
        .collect();

    Ok(AspectScan { rows, pending })
}

/// 把量到的像素尺寸写回库，返回写入条数。
pub fn store_dimensions(conn: &mut Connection, measured: &[(i64, u32, u32)]) -> AppResult<usize> {
    if measured.is_empty() {
        return Ok(0);
    }

    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare("UPDATE artworks SET width = ?1, height = ?2 WHERE id = ?3")?;
        for (id, width, height) in measured {
            stmt.execute(params![*width as i64, *height as i64, id])?;
        }
    }
    tx.commit()?;

    Ok(measured.len())
}

/// 量一张图优先读哪个文件。
///
/// 缩略图与原件长宽比一致（等比缩放），但它小得多、必然躺在本地缓存目录里，
/// 读头更快也更可靠。缩略图路径为空时才退回原图。
fn pick_probe(thumbnail: Option<&str>, original: &str) -> PathBuf {
    match thumbnail {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => PathBuf::from(original),
    }
}

/// 并行读一批图片的像素尺寸，返回与输入同序的结果。
///
/// `image::image_dimensions` 只读文件头（单张约 1~3ms），串行跑四千多张仍要好几秒；
/// 摊到多线程只剩几百毫秒。这一步**只发生一次**（回填完就有值了），
/// 所以不为它引入任何常驻缓存。
pub fn measure_dimensions(jobs: &[PathBuf]) -> Vec<Option<(u32, u32)>> {
    if jobs.is_empty() {
        return Vec::new();
    }

    let threads = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .clamp(1, 8)
        .min(jobs.len());

    let cursor = std::sync::atomic::AtomicUsize::new(0);
    let out = std::sync::Mutex::new(vec![None; jobs.len()]);

    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let index = cursor.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if index >= jobs.len() {
                    break;
                }
                out.lock().unwrap()[index] = image::image_dimensions(&jobs[index]).ok();
            });
        }
    });

    out.into_inner().unwrap()
}

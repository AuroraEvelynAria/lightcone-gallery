//! 文件系统扫描：递归发现图片、读取元数据，并与数据库做增量同步。
//!
//! 增量策略：先把该图库的现存记录读成内存快照（path -> (size, mtime)），
//! 再逐文件对比 —— 只有新增 / 变化的文件才写库，最后清掉磁盘上已消失的记录。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rusqlite::{Connection, params};
use walkdir::WalkDir;

use crate::db::models::{ScanPhase, ScanProgress, ScanSummary};
use crate::db::now_unix;
use crate::error::{AppError, AppResult};

/// `image` crate 已启用的解码格式对应的扩展名。
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "ico",
];

/// 递归时直接跳过的目录名（版本控制、依赖、系统回收站等）。
const SKIPPED_DIRS: &[&str] = &[
    ".git",
    ".svn",
    "node_modules",
    "target",
    "$RECYCLE.BIN",
    "System Volume Information",
];

pub fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| SUPPORTED_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn should_descend(entry: &walkdir::DirEntry) -> bool {
    if !entry.file_type().is_dir() || entry.depth() == 0 {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    !name.starts_with('.') && !SKIPPED_DIRS.iter().any(|skip| skip.eq_ignore_ascii_case(&name))
}

fn modified_unix(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|dur| dur.as_secs() as i64)
        .unwrap_or(0)
}

struct Discovered {
    path: String,
    filename: String,
    extension: String,
    file_size: i64,
    modified_at: i64,
    width: Option<i64>,
    height: Option<i64>,
}

/// 扫描一个图库根目录，把结果增量写回数据库。
///
/// `on_progress` 会收到发现阶段与索引阶段的回调，调用方负责转发为前端事件。
pub fn scan_library(
    conn: &mut Connection,
    library_id: i64,
    library_name: &str,
    root: &Path,
    on_progress: &mut dyn FnMut(ScanProgress),
) -> AppResult<ScanSummary> {
    if !root.is_dir() {
        return Err(AppError::InvalidPath(format!(
            "目录不存在或不可访问: {}",
            root.display()
        )));
    }

    let mut progress = |phase: ScanPhase, processed: usize, total: usize| {
        on_progress(ScanProgress {
            library_id,
            library_name: library_name.to_string(),
            phase,
            processed,
            total,
        });
    };

    // ---- 阶段一：发现文件 -------------------------------------------------
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(should_descend)
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file() && is_supported_image(entry.path()) {
            files.push(entry.path().to_path_buf());
            if files.len() % 256 == 0 {
                progress(ScanPhase::Discovering, files.len(), 0);
            }
        }
    }
    let total = files.len();
    progress(ScanPhase::Discovering, total, total);

    // ---- 阶段二：读取元数据 -----------------------------------------------
    let mut skipped = 0usize;
    let mut discovered: Vec<Discovered> = Vec::with_capacity(total);

    for (index, path) in files.iter().enumerate() {
        let Some(item) = probe(path) else {
            skipped += 1;
            continue;
        };
        discovered.push(item);

        if index % 64 == 0 {
            progress(ScanPhase::Indexing, index, total);
        }
    }

    // ---- 阶段三：增量写库 -------------------------------------------------
    let existing = load_snapshot(conn, library_id)?;
    let seen: HashSet<&str> = discovered.iter().map(|d| d.path.as_str()).collect();
    let now = now_unix();

    let mut added = 0usize;
    let mut updated = 0usize;
    let mut unchanged = 0usize;
    let mut removed = 0usize;

    let tx = conn.transaction()?;
    {
        let mut upsert = tx.prepare(
            "INSERT INTO images
                 (library_id, path, filename, extension, width, height,
                  file_size, modified_at, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(path) DO UPDATE SET
                 library_id  = excluded.library_id,
                 filename    = excluded.filename,
                 extension   = excluded.extension,
                 width       = excluded.width,
                 height      = excluded.height,
                 file_size   = excluded.file_size,
                 modified_at = excluded.modified_at,
                 indexed_at  = excluded.indexed_at",
        )?;

        for item in &discovered {
            match existing.get(&item.path) {
                Some(&(size, mtime)) if size == item.file_size && mtime == item.modified_at => {
                    unchanged += 1;
                    continue;
                }
                Some(_) => updated += 1,
                None => added += 1,
            }

            upsert.execute(params![
                library_id,
                item.path,
                item.filename,
                item.extension,
                item.width,
                item.height,
                item.file_size,
                item.modified_at,
                now,
            ])?;
        }

        // 清理磁盘上已消失的记录（含被重命名走的旧路径）
        let mut prune = tx.prepare("DELETE FROM images WHERE library_id = ?1 AND path = ?2")?;
        for path in existing.keys() {
            if !seen.contains(path.as_str()) {
                removed += prune.execute(params![library_id, path])?;
            }
        }
    }

    tx.execute(
        "UPDATE libraries SET last_scanned_at = ?1 WHERE id = ?2",
        params![now, library_id],
    )?;
    tx.commit()?;

    progress(ScanPhase::Finished, total, total);

    Ok(ScanSummary {
        library_id,
        discovered: total,
        added,
        updated,
        unchanged,
        removed,
        skipped,
    })
}

/// 读取文件头拿到尺寸等元数据；读不出说明当前没有对应解码器，返回 None。
fn probe(path: &Path) -> Option<Discovered> {
    let meta = std::fs::metadata(path).ok()?;
    // `image_dimensions` 只解析文件头，不解码整张图，因此可以放心遍历大目录
    let (width, height) = image::image_dimensions(path).ok()?;

    Some(Discovered {
        path: path.to_string_lossy().into_owned(),
        filename: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        extension: path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default(),
        file_size: meta.len() as i64,
        modified_at: modified_unix(&meta),
        width: Some(width as i64),
        height: Some(height as i64),
    })
}

type Snapshot = HashMap<String, (i64, i64)>;

fn load_snapshot(conn: &Connection, library_id: i64) -> AppResult<Snapshot> {
    let mut stmt =
        conn.prepare("SELECT path, file_size, modified_at FROM images WHERE library_id = ?1")?;
    let rows = stmt.query_map([library_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            (row.get::<_, i64>(1)?, row.get::<_, i64>(2)?),
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<Snapshot>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn make_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("lightcone-scan-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_image(path: &Path, width: u32, height: u32) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        image::RgbImage::new(width, height).save(path).unwrap();
    }

    fn open_indexed_db(root: &Path) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        schema::migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO libraries (id, path, name, created_at) VALUES (1, ?1, 'test', 0)",
            [root.to_string_lossy().to_string()],
        )
        .unwrap();
        conn
    }

    #[test]
    fn extension_filter_is_case_insensitive() {
        assert!(is_supported_image(Path::new("C:/a/photo.JPG")));
        assert!(is_supported_image(Path::new("C:/a/photo.jpeg")));
        assert!(!is_supported_image(Path::new("C:/a/clip.mp4")));
        assert!(!is_supported_image(Path::new("C:/a/README")));
    }

    /// 覆盖核心链路：首次索引 -> 无变化重扫 -> 文件消失后清理。
    #[test]
    fn scan_indexes_then_detects_changes_and_removals() {
        let root = make_root("cycle");
        write_image(&root.join("a.png"), 8, 8);
        write_image(&root.join("nested/deep/b.jpg"), 4, 6);
        std::fs::write(root.join("notes.txt"), b"not an image").unwrap();

        let mut conn = open_indexed_db(&root);
        let mut events = Vec::new();

        let first = scan_library(&mut conn, 1, "test", &root, &mut |p| events.push(p)).unwrap();
        assert_eq!(first.discovered, 2, "只应发现两张图片，.txt 必须被忽略");
        assert_eq!(first.added, 2);
        assert_eq!(first.unchanged, 0);
        assert_eq!(first.skipped, 0);
        assert!(
            events.iter().any(|e| matches!(e.phase, ScanPhase::Finished)),
            "应发出 Finished 阶段事件"
        );

        let (width, height): (i64, i64) = conn
            .query_row(
                "SELECT width, height FROM images WHERE filename = 'a.png'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((width, height), (8, 8), "应记录图片真实尺寸");

        // 第二次扫描：文件没动过，不应重复写入
        let second = scan_library(&mut conn, 1, "test", &root, &mut |_| {}).unwrap();
        assert_eq!(second.added, 0);
        assert_eq!(second.unchanged, 2);

        // 删掉一张后重扫：记录应被清理
        std::fs::remove_file(root.join("a.png")).unwrap();
        let third = scan_library(&mut conn, 1, "test", &root, &mut |_| {}).unwrap();
        assert_eq!(third.removed, 1);

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM images", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_rejects_missing_directory() {
        let conn = Connection::open_in_memory().unwrap();
        let mut conn = conn;
        let missing = std::env::temp_dir().join("lightcone-does-not-exist-xyz");
        assert!(scan_library(&mut conn, 1, "test", &missing, &mut |_| {}).is_err());
    }
}

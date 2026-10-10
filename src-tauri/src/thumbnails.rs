//! 缩略图生成与磁盘缓存。
//!
//! 缓存布局： `<app_data>/thumbnails/<owner>/<size>/<id>.webp`
//!
//! 相比通用图库时期有两处关键变化：
//!
//! 1. 统一输出**有损 WebP**。`image` crate 自带的 WebP 编码器只实现了 VP8L 无损，
//!    无损压照片会比 JPEG 更大，做缩略图没有意义，因此这里改用 `webp`（libwebp 绑定）
//!    走有损通道。顺带解决了老实现把 PNG / ICO 透明区域压掉的问题。
//! 2. 缓存目录按归属域 `owner` 分层。`images.id` 与 `artworks.id` 是两张表各自的自增
//!    主键，共用同一层目录会撞车（`images#1` 与 `artworks#1` 会落到同一个文件上）。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::{AppError, AppResult};

/// 允许请求的缩略图边长白名单，避免任意 size 撑爆缓存目录。
pub const ALLOWED_SIZES: &[u32] = &[160, 320, 400, 640];
pub const DEFAULT_SIZE: u32 = 320;

/// 作品库缩略图的目标边长。Pixiv 作品以插画为主，400 在常见格子尺寸下
/// 即使按 1.5~2 倍屏缩放也不会糊。
pub const ARTWORK_SIZE: u32 = 400;

/// 渲染线程数上限。再往上收益递减，而且会把 CPU 从 UI 那边抢走。
const MAX_RENDER_THREADS: usize = 8;

/// 有损 WebP 质量。缩略图尺度上 80 已看不出差别；实测同一张照片，
/// WebP q80 约为 JPEG q82 的 80% 体积。
const WEBP_QUALITY: f32 = 80.0;

/// 缩略图的归属域，决定缓存落在哪个子目录。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// 通用图库的 `images.id`
    Image,
    /// Pixiv 作品维度的 `artworks.id`
    Artwork,
}

impl Owner {
    fn dir_name(self) -> &'static str {
        match self {
            Owner::Image => "images",
            Owner::Artwork => "artworks",
        }
    }
}

pub fn normalize_size(size: u32) -> u32 {
    if ALLOWED_SIZES.contains(&size) {
        size
    } else {
        DEFAULT_SIZE
    }
}

pub fn thumbnail_path(cache_dir: &Path, owner: Owner, id: i64, size: u32) -> PathBuf {
    cache_dir
        .join(owner.dir_name())
        .join(size.to_string())
        .join(format!("{id}.webp"))
}

/// 返回缩略图路径，必要时从原图生成。
///
/// 缓存命中条件是「缩略图存在且不比原图旧」，因此原图被替换后会自动重算。
pub fn ensure_thumbnail(
    cache_dir: &Path,
    owner: Owner,
    id: i64,
    source: &Path,
    size: u32,
) -> AppResult<PathBuf> {
    let size = normalize_size(size);
    let dest = thumbnail_path(cache_dir, owner, id, size);

    if is_fresh(&dest, source) {
        return Ok(dest);
    }

    let bytes = render(source, size)?;

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&dest, &bytes)?;

    Ok(dest)
}

/// 校验源文件存在并生成 / 复用缓存，全程丢到阻塞线程池。
///
/// 命令层的 `thumbnail_for` 与 `thumbnail_for_artwork` 只有「按 id 取源路径」的 SQL
/// 不同，其余流水线共用这里，省得两边各写一遍 `spawn_blocking` 与存在性检查。
pub async fn ensure_offloaded(
    cache_dir: PathBuf,
    owner: Owner,
    id: i64,
    source: PathBuf,
    size: u32,
) -> AppResult<String> {
    tauri::async_runtime::spawn_blocking(move || {
        if !source.is_file() {
            return Err(AppError::NotFound(format!(
                "源文件已不存在: {}",
                source.display()
            )));
        }
        let path = ensure_thumbnail(&cache_dir, owner, id, &source, size)?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|err| AppError::Other(format!("缩略图任务执行失败: {err}")))?
}

/// 缓存是否可直接用：文件存在，且不早于原图。
///
/// 扫描阶段的批量生成靠它跳过已经算过的，这样重复扫同一目录几乎不花时间。
pub fn is_cached(cache_dir: &Path, owner: Owner, id: i64, source: &Path, size: u32) -> bool {
    is_fresh(
        &thumbnail_path(cache_dir, owner, id, normalize_size(size)),
        source,
    )
}

/// 并行生成一批缩略图，返回成功落盘的 `(id, 缩略图路径)`。
///
/// 解码 / 缩放 / 编码全在 CPU 上，实测单张约 23ms：串行跑一万张要近四分钟，
/// 所以摊到多个线程（libwebp 对互不相关的图像编解码是线程安全的）。
///
/// SQLite 写入**刻意不在这里做** —— 调用方拿到结果后自己串行落库，
/// 免得把连接和锁带进线程池。
///
/// `on_progress` 每完成一张调一次，节流与上报由调用方负责。
pub fn ensure_parallel(
    cache_dir: &Path,
    owner: Owner,
    jobs: &[(i64, PathBuf)],
    size: u32,
    on_progress: &(dyn Fn(usize, usize) + Sync),
) -> Vec<(i64, PathBuf)> {
    let total = jobs.len();
    if total == 0 {
        return Vec::new();
    }

    let size = normalize_size(size);
    let threads = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .clamp(1, MAX_RENDER_THREADS)
        .min(total);

    let cursor = AtomicUsize::new(0);
    let finished = AtomicUsize::new(0);
    let rendered = Mutex::new(Vec::with_capacity(total));

    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    // 原子游标领活：谁抢到算谁的，天然负载均衡
                    let index = cursor.fetch_add(1, Ordering::Relaxed);
                    if index >= total {
                        break;
                    }

                    let (id, source) = &jobs[index];
                    if let Ok(path) = ensure_thumbnail(cache_dir, owner, *id, source, size) {
                        rendered.lock().unwrap().push((*id, path));
                    }

                    // fetch_add 的返回值唯一，所以每个完成数只会被一个线程看到
                    let done = finished.fetch_add(1, Ordering::Relaxed) + 1;
                    on_progress(done, total);
                }
            });
        }
    });

    rendered.into_inner().unwrap()
}

/// 解码 -> 等比缩放 -> 有损 WebP 编码，返回文件字节。
fn render(source: &Path, size: u32) -> AppResult<Vec<u8>> {
    let decoded = image::open(source)?;
    // `thumbnail` 等比缩放到不超出 size×size，且不会放大小图
    let resized = decoded.thumbnail(size, size);

    let encoded = if resized.color().has_alpha() {
        // WebP 支持 alpha，带透明通道的源图（PNG / ICO）不必再压到 RGB
        let raw = resized.to_rgba8();
        webp::Encoder::from_rgba(raw.as_raw(), raw.width(), raw.height()).encode(WEBP_QUALITY)
    } else {
        let raw = resized.to_rgb8();
        webp::Encoder::from_rgb(raw.as_raw(), raw.width(), raw.height()).encode(WEBP_QUALITY)
    };

    Ok(encoded.to_vec())
}

/// 缩略图存在、且修改时间不早于原图即视为可用。
fn is_fresh(dest: &Path, source: &Path) -> bool {
    let (Ok(dest_meta), Ok(src_meta)) = (fs::metadata(dest), fs::metadata(source)) else {
        return false;
    };
    match (dest_meta.modified(), src_meta.modified()) {
        (Ok(dest_time), Ok(src_time)) => dest_time >= src_time,
        _ => false,
    }
}

/// 统计缓存占用（字节数）与文件数，供设置页展示 / 清理前确认。
pub fn cache_usage(cache_dir: &Path) -> (u64, u64) {
    let mut bytes = 0u64;
    let mut count = 0u64;

    for entry in walkdir::WalkDir::new(cache_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file()
            && let Ok(meta) = entry.metadata()
        {
            bytes += meta.len();
            count += 1;
        }
    }

    (bytes, count)
}

/// 清空整个缩略图缓存；下次浏览时会按需重建。
pub fn clear_cache(cache_dir: &Path) -> AppResult<()> {
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个干净的一次性工作目录，测试结束由调用方清理。
    fn scratch(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "lightcone-thumb-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn unknown_sizes_fall_back_to_default() {
        assert_eq!(normalize_size(160), 160);
        assert_eq!(normalize_size(9999), DEFAULT_SIZE);
        assert_eq!(normalize_size(0), DEFAULT_SIZE);
    }

    #[test]
    fn cache_path_is_partitioned_by_size_and_owner() {
        let cache = Path::new("C:/cache");
        let image = thumbnail_path(cache, Owner::Image, 42, 320);
        let artwork = thumbnail_path(cache, Owner::Artwork, 42, 320);

        assert!(image.ends_with("320/42.webp") || image.ends_with(r"320\42.webp"));
        assert!(image.to_string_lossy().contains("images"));
        // 两张表的自增 id 会撞车，必须落在不同目录，否则会互相覆盖缩略图
        assert_ne!(image, artwork, "不同归属域的同一 id 不能共用缓存文件");
    }

    /// 真实解码一张 PNG，覆盖「生成 -> 复用缓存 -> 原图变化后重算」的完整链路。
    #[test]
    fn ensure_thumbnail_generates_reuses_and_invalidates_cache() {
        let root = scratch("basic");
        let cache = root.join("cache");

        let source = root.join("photo.png");
        image::RgbImage::new(1200, 800).save(&source).unwrap();

        let path = ensure_thumbnail(&cache, Owner::Image, 7, &source, 320).unwrap();
        assert!(path.is_file(), "应生成缩略图文件");

        // 输出必须是 WebP：RIFF....WEBP
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF", "缩略图应是 RIFF 容器");
        assert_eq!(&bytes[8..12], b"WEBP", "缩略图应是 WebP 编码");

        let (width, height) = image::image_dimensions(&path).unwrap();
        assert!(width <= 320 && height <= 320, "缩略图不得超过请求边长");
        let ratio = f64::from(width) / f64::from(height);
        assert!((ratio - 1.5).abs() < 0.02, "应保持 3:2 宽高比，实际 {width}x{height}");

        // 缓存命中：第二次调用不应重写文件
        let mtime_before = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert_eq!(ensure_thumbnail(&cache, Owner::Image, 7, &source, 320).unwrap(), path);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            mtime_before,
            "缓存未过期时不应重新生成"
        );

        // 原图比缓存新 -> 必须重算
        let handle = std::fs::OpenOptions::new().write(true).open(&source).unwrap();
        handle
            .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();
        drop(handle);

        let regenerated = ensure_thumbnail(&cache, Owner::Image, 7, &source, 320).unwrap();
        assert!(
            std::fs::metadata(&regenerated).unwrap().modified().unwrap() > mtime_before,
            "原图更新后应重新生成缩略图"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 老实现把缩略图压成 JPEG，透明通道会丢；换 WebP 后必须保住。
    #[test]
    fn ensure_thumbnail_keeps_alpha_channel() {
        let root = scratch("alpha");
        let cache = root.join("cache");

        let source = root.join("sticker.png");
        let mut rgba = image::RgbaImage::new(64, 64);
        for (_, _, pixel) in rgba.enumerate_pixels_mut() {
            *pixel = image::Rgba([10, 200, 90, 128]);
        }
        rgba.save(&source).unwrap();

        let path = ensure_thumbnail(&cache, Owner::Image, 1, &source, 320).unwrap();
        let decoded = image::open(&path).unwrap();
        assert!(
            decoded.color().has_alpha(),
            "WebP 缩略图应保留透明通道，实际为 {:?}",
            decoded.color()
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 同一 id 在两个归属域下互不干扰，这是分目录的核心目的。
    #[test]
    fn same_id_in_two_owners_does_not_collide() {
        let root = scratch("owners");
        let cache = root.join("cache");

        let source = root.join("photo.png");
        image::RgbImage::new(400, 400).save(&source).unwrap();

        let as_image = ensure_thumbnail(&cache, Owner::Image, 1, &source, 160).unwrap();
        let as_artwork = ensure_thumbnail(&cache, Owner::Artwork, 1, &source, 160).unwrap();

        assert_ne!(as_image, as_artwork);
        assert!(as_image.is_file() && as_artwork.is_file());

        let _ = std::fs::remove_dir_all(&root);
    }
}

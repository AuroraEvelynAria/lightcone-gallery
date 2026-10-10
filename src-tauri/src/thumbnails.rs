//! 缩略图生成与磁盘缓存。
//!
//! 缓存布局： `<app_data>/thumbnails/<size>/<image_id>.jpg`
//! 统一输出 JPEG —— 图库主体是照片，JPEG 在体积和解码速度上最均衡。
//! 副作用：带透明通道的源图（PNG/ICO）会被压到 RGB，透明区域按源像素值保留。

use std::fs;
use std::path::{Path, PathBuf};

use image::codecs::jpeg::JpegEncoder;

use crate::error::AppResult;

/// 允许请求的缩略图边长白名单，避免任意 size 撑爆缓存目录。
pub const ALLOWED_SIZES: &[u32] = &[160, 320, 640];
pub const DEFAULT_SIZE: u32 = 320;

const JPEG_QUALITY: u8 = 82;

pub fn normalize_size(size: u32) -> u32 {
    if ALLOWED_SIZES.contains(&size) {
        size
    } else {
        DEFAULT_SIZE
    }
}

pub fn thumbnail_path(cache_dir: &Path, image_id: i64, size: u32) -> PathBuf {
    cache_dir
        .join(size.to_string())
        .join(format!("{image_id}.jpg"))
}

/// 返回缩略图路径，必要时从原图生成。
///
/// 缓存命中条件是「缩略图存在且不比原图旧」，因此原图被替换后会自动重算。
pub fn ensure_thumbnail(
    cache_dir: &Path,
    image_id: i64,
    source: &Path,
    size: u32,
) -> AppResult<PathBuf> {
    let size = normalize_size(size);
    let dest = thumbnail_path(cache_dir, image_id, size);

    if is_fresh(&dest, source) {
        return Ok(dest);
    }

    let decoded = image::open(source)?;
    // `thumbnail` 等比缩放到不超出 size×size，且不会放大小图
    let resized = decoded.thumbnail(size, size).to_rgb8();

    let mut buffer: Vec<u8> = Vec::new();
    JpegEncoder::new_with_quality(&mut buffer, JPEG_QUALITY).encode_image(&resized)?;

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&dest, &buffer)?;

    Ok(dest)
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

    #[test]
    fn unknown_sizes_fall_back_to_default() {
        assert_eq!(normalize_size(160), 160);
        assert_eq!(normalize_size(9999), DEFAULT_SIZE);
        assert_eq!(normalize_size(0), DEFAULT_SIZE);
    }

    #[test]
    fn cache_path_is_partitioned_by_size() {
        let path = thumbnail_path(Path::new("C:/cache"), 42, 320);
        assert!(path.ends_with("320/42.jpg") || path.ends_with("320\\42.jpg"));
    }

    /// 真实解码一张 PNG，覆盖「生成 -> 复用缓存 -> 原图变化后重算」的完整链路。
    #[test]
    fn ensure_thumbnail_generates_reuses_and_invalidates_cache() {
        let root = std::env::temp_dir().join(format!("lightcone-thumb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let cache = root.join("cache");
        std::fs::create_dir_all(&root).unwrap();

        let source = root.join("photo.png");
        image::RgbImage::new(1200, 800).save(&source).unwrap();

        let path = ensure_thumbnail(&cache, 7, &source, 320).unwrap();
        assert!(path.is_file(), "应生成缩略图文件");

        let (width, height) = image::image_dimensions(&path).unwrap();
        assert!(width <= 320 && height <= 320, "缩略图不得超过请求边长");
        let ratio = f64::from(width) / f64::from(height);
        assert!((ratio - 1.5).abs() < 0.02, "应保持 3:2 宽高比，实际 {width}x{height}");

        // 缓存命中：第二次调用不应重写文件
        let mtime_before = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert_eq!(ensure_thumbnail(&cache, 7, &source, 320).unwrap(), path);
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

        let regenerated = ensure_thumbnail(&cache, 7, &source, 320).unwrap();
        assert!(
            std::fs::metadata(&regenerated).unwrap().modified().unwrap() > mtime_before,
            "原图更新后应重新生成缩略图"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}

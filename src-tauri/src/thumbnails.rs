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

/// 作品库缩略图的目标边长。
///
/// **400 曾经够用，直到手机上出现一列满宽的格子。** 手机屏幕密度普遍到 3.25
/// （1220×2712 @520dpi），一列铺满时格子的**物理**边长能到 1084px；而卡片是正方
/// `object-cover` —— 裁切吃的是缩略图的**短边**，400 长边的 3:2 图短边只有 267px，
/// 等于被放大四倍，肉眼可见地糊。桌面默认窗口下格子约 358 物理像素、
/// 短边 267 只放大 1.34 倍，所以这个毛病只在手机侧暴露得出来。
///
/// 640 是权衡后的取值：手机改两列后格子约 522 物理像素，短边 427 只需放大 1.22 倍
/// （3.25 的屏密度下已看不出差别）。代价是缓存体积约为 400 的 2.56 倍
/// —— 实测 4430 张从 91MB 涨到约 207MB，相对原图库可以接受。
pub const ARTWORK_SIZE: u32 = 640;

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

/// 库里存的缩略图路径是否指向**当前尺寸**的缓存目录。
///
/// 缓存按 `<owner>/<size>/` 分层，所以改动 `ARTWORK_SIZE` 之后，`artworks.thumbnail_path`
/// 里存的还是旧尺寸的路径 —— 文件确实在、`<img>` 也打得开，只是分辨率不够，
/// 构造不出任何报错（这正是它危险的地方：不报错，只是糊）。
///
/// 判成「不当前」之后由调用方决定怎么办：命令层把它当 `None` 返回，前端就会走一次
/// 重生成回退（`thumbnail_for_artwork`，那边用的是当前的 `ARTWORK_SIZE`），
/// 顺手把新尺寸补上并写回库 —— 于是主人滑到哪张补哪张，不必重扫整个库。
pub fn is_current_size(stored: Option<&str>, size: u32) -> bool {
    let Some(dir) = stored.map(Path::new).and_then(Path::parent) else {
        return false;
    };
    let wanted = size.to_string();
    dir.file_name().and_then(|name| name.to_str()) == Some(wanted.as_str())
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

    // 新尺寸已经落盘，旧尺寸的那一份从此不会再被读到（列表已经把它的路径判成过期）。
    // 不删的话换一次尺寸就白留一整份 —— 本机实测 4430 张是 91MB，
    // 而手机上的空间更金贵。放在这里而不是命令层，是为了让「单张回退」与
    // 「扫描时的批量生成」两条路都自动收敛，不必各写一遍。
    remove_other_sizes(cache_dir, owner, id, size);

    Ok(dest)
}

/// 删掉同一个 id 在**别的尺寸目录**下留下的缓存文件，返回删掉几个。
///
/// 只认 `<cache>/<owner>/<纯数字>/<id>.webp` 这一种形状：目录名必须能解析成数字、
/// 文件名必须正好是 `<id>.webp`。任何对不上的都跳过 —— 这个函数会删文件，
/// 判据就得窄到「手滑也删不到别的东西」（原图根本不在缓存目录下，够不着）。
pub fn remove_other_sizes(cache_dir: &Path, owner: Owner, id: i64, keep: u32) -> usize {
    let Ok(entries) = fs::read_dir(cache_dir.join(owner.dir_name())) else {
        return 0;
    };

    let mut removed = 0;
    for entry in entries.filter_map(Result::ok) {
        let dir = entry.path();
        let other = dir
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.parse::<u32>().ok());
        if other.is_none() || other == Some(keep) {
            continue;
        }
        if fs::remove_file(dir.join(format!("{id}.webp"))).is_ok() {
            removed += 1;
        }
    }
    removed
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

    /// 改了 `ARTWORK_SIZE` 之后，库里存的旧尺寸路径必须能被认出来。
    ///
    /// 这条判据是「滑到哪张补哪张」整套懒重算的**唯一入口**：判不出来，旧图就永远
    /// 服务下去（文件在、能打开，只是糊，不会有任何报错）；判得太宽，每次列出一页
    /// 都会把好端端的缩略图判成缺失、白白重算一遍。
    #[test]
    fn current_size_predicate_matches_only_the_active_size_dir() {
        let old = format!("C:/cache/artworks/{}/1.webp", ARTWORK_SIZE / 2);
        let current = format!("C:/cache/artworks/{ARTWORK_SIZE}/1.webp");

        assert!(is_current_size(Some(&current), ARTWORK_SIZE), "当前尺寸要认");
        assert!(!is_current_size(Some(&old), ARTWORK_SIZE), "旧尺寸目录要判过期");
        assert!(
            !is_current_size(None, ARTWORK_SIZE),
            "库里为 NULL 的不能当成命中"
        );
        assert!(
            !is_current_size(Some(""), ARTWORK_SIZE),
            "空串不能当真：它在 Windows 上会被解析成当前目录，父目录名是随机的"
        );
        // 只有目录名参与比较，文件名不该影响结论
        assert!(is_current_size(
            Some(&format!("C:/cache/artworks/{ARTWORK_SIZE}/999.webp")),
            ARTWORK_SIZE
        ));
        // 尺寸串出现在路径中间（而非末级目录）不算数，避免用 starts_with 之类的宽判据
        assert!(
            !is_current_size(
                Some(&format!("C:/cache/{ARTWORK_SIZE}/artworks/1.webp")),
                ARTWORK_SIZE
            ),
            "尺寸必须是末级目录 —— 路径里别处撞上同一个数字不算命中"
        );
    }

    /// 换尺寸时要顺手把旧尺寸的副本删掉，但**只能删该删的**。
    ///
    /// 这个函数会真的删文件，所以判据必须窄：目录名解析不出数字不删、不是当前 id
    /// 的不删、名字不叫 `<id>.webp` 的不删、别的归属域不碰、当前尺寸本身更不能删。
    #[test]
    fn remove_other_sizes_only_touches_the_matching_copy() {
        let root = scratch("prune");
        let cache = root.join("cache");

        let write = |relative: &str| {
            let path = cache.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"x").unwrap();
            path
        };

        let stale = write("artworks/400/7.webp");
        let keep = write("artworks/640/7.webp");
        let odd_dir = write("artworks/not-a-size/7.webp");
        let other_id = write("artworks/400/8.webp");
        let other_ext = write("artworks/400/7.png");
        let other_owner = write("images/400/7.webp");

        assert_eq!(
            remove_other_sizes(&cache, Owner::Artwork, 7, 640),
            1,
            "只有 artworks/400/7.webp 该被删"
        );

        assert!(!stale.exists(), "旧尺寸的副本应被删掉");
        assert!(keep.exists(), "当前尺寸绝不能被删");
        assert!(odd_dir.exists(), "目录名不解析成数字就跳过，不猜");
        assert!(other_id.exists(), "别的 id 不能误伤");
        assert!(other_ext.exists(), "文件名必须正好是 <id>.webp");
        assert!(other_owner.exists(), "别的归属域不能碰（两张表的 id 会撞车）");

        let _ = fs::remove_dir_all(&root);
    }

    /// 走一遍真实生成：先出 160 的，再出 320 的，前者应被自动清掉。
    #[test]
    fn regenerating_at_another_size_prunes_the_old_copy() {
        let root = scratch("prune-integration");
        let cache = root.join("cache");

        let source = root.join("photo.png");
        image::RgbImage::new(800, 600).save(&source).unwrap();

        let small = ensure_thumbnail(&cache, Owner::Artwork, 3, &source, 160).unwrap();
        assert!(small.is_file());

        let large = ensure_thumbnail(&cache, Owner::Artwork, 3, &source, 320).unwrap();
        assert!(large.is_file());
        assert!(
            !small.exists(),
            "升到 320 之后，160 的那份已经无人引用，不该留在盘上"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// 真实图片上的缩略图耗时基准（默认忽略）。
    ///
    /// **为什么要分段量**：目标边长不是唯一的成本项 —— 解码原图是大头，而且
    /// **与目标尺寸无关**（`image` 0.25.10 / zune-jpeg 0.5.15 都没暴露 JPEG 的缩放
    /// 解码，只能全解一次再缩）。改尺寸只影响「缩放 + 编码」这一段。不把两段分开，
    /// 就没法判断放大到 640 到底贵多少（也容易被「换尺寸后整轮扫描慢了多少」
    /// 这种掺了解码的读数带偏）。
    ///
    /// 本机 release 实测（40 张 4000px 级的 pixez 原图，8 核）：
    /// 解码 121ms、渲染 400 共 159ms、渲染 640 共 176ms —— 放大到 640 净增 17.5ms
    /// （+11%），体积 17.5KB → 34.2KB（×1.95）。值得。
    ///
    /// 跑法（指向任意一个装着原图的目录，只读，不写缓存）：
    /// ```text
    /// LIGHTCONE_BENCH_DIR="D:/pixez pics/pixez/某位画师" \
    ///   cargo test --lib bench_real_thumbnails -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "需要真实图库：设 LIGHTCONE_BENCH_DIR 后加 --ignored 跑"]
    fn bench_real_thumbnails() {
        use std::time::{Duration, Instant};

        let dir = std::env::var("LIGHTCONE_BENCH_DIR")
            .expect("请设 LIGHTCONE_BENCH_DIR 指向一个装着原图的目录");
        let mut sources: Vec<PathBuf> = fs::read_dir(&dir)
            .expect("读不到该目录")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && matches!(
                        path.extension()
                            .and_then(|ext| ext.to_str())
                            .map(str::to_ascii_lowercase)
                            .as_deref(),
                        Some("jpg" | "jpeg" | "png" | "webp")
                    )
            })
            .collect();
        sources.sort();
        sources.truncate(40);
        assert!(!sources.is_empty(), "目录里没有可解码的图片: {dir}");

        let median = |mut samples: Vec<Duration>| -> f64 {
            samples.sort();
            samples[samples.len() / 2].as_secs_f64() * 1000.0
        };

        let (mut decode, mut small, mut large) = (Vec::new(), Vec::new(), Vec::new());
        let (mut small_bytes, mut large_bytes) = (0usize, 0usize);

        for source in &sources {
            let started = Instant::now();
            let _ = image::open(source).expect("样图解不开");
            decode.push(started.elapsed());

            let started = Instant::now();
            small_bytes += render(source, 400).expect("渲染失败").len();
            small.push(started.elapsed());

            let started = Instant::now();
            large_bytes += render(source, ARTWORK_SIZE).expect("渲染失败").len();
            large.push(started.elapsed());
        }

        let count = sources.len() as f64;
        let decode_ms = median(decode);
        let small_ms = median(small);
        let large_ms = median(large);
        println!("样本 {count} 张（目录 {dir}）");
        println!("  纯解码原图        {decode_ms:>7.1} ms   ← 与目标尺寸无关，是流水线的大头");
        println!("  渲染 400          {small_ms:>7.1} ms   ← 含解码");
        println!("  渲染 {ARTWORK_SIZE}          {large_ms:>7.1} ms   ← 含解码");
        println!(
            "  放大到 {ARTWORK_SIZE} 的净增   {:>7.1} ms/张（缩放+编码这一段）",
            large_ms - small_ms
        );
        println!(
            "  平均体积          400: {:>6.1} KB   {ARTWORK_SIZE}: {:>6.1} KB   （×{:.2}）",
            small_bytes as f64 / count / 1024.0,
            large_bytes as f64 / count / 1024.0,
            large_bytes as f64 / small_bytes as f64
        );
    }
}

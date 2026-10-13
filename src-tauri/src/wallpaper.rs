//! 把一张图设为系统桌面壁纸。
//!
//! 两端走完全不同的路：Windows 上是 `SystemParametersInfoW` 一句 FFI，
//! Android 上交给 Kotlin 的 `WallpaperManager`（见 `MediaStoreBridge`）。
//! 这里只负责 Windows，以及两端共用的「格式能不能直接用」判断。

use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};

/// Windows 认得、可以直接喂给 `SystemParametersInfoW` 的扩展名。
///
/// 清单来自微软的壁纸支持格式（Windows 8 起从「只认 BMP」放宽到这些）。
/// 不在其中的（WebP、ICO 这些）**必须先转码**：直接传进去 Windows 会
/// **静默失败** —— 返回成功、桌面却不变，是最难查的那种故障。
const NATIVE_EXTENSIONS: &[&str] = &[
    "bmp", "dib", "jpg", "jpeg", "jfif", "png", "gif", "tif", "tiff",
];

/// 要设成哪一张壁纸。
///
/// 与前端 `api.setWallpaper` 的 `target` 一一对应，两端语义保持一致：
/// 手机上三个都能设，桌面上 `lock` / `both` 走 WinRT。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// 桌面（手机上叫「主屏幕」）
    Home,
    /// 锁屏
    Lock,
    /// 两个都设
    Both,
}

impl Target {
    /// 解析前端传来的字符串，认不出的按「桌面」处理。
    pub fn parse(raw: Option<&str>) -> Self {
        match raw {
            Some("lock") => Self::Lock,
            Some("both") => Self::Both,
            _ => Self::Home,
        }
    }
}

/// 设为壁纸。
///
/// `cache_dir` 用来放转码后的 PNG —— 只有格式不被 Windows 接受时才会用到。
/// 调用方负责把它放在**应用数据目录**而不是临时目录：注册表里存的是这个路径，
/// 重启之后还得读得到。
pub fn set(path: &Path, cache_dir: &Path, target: Target) -> AppResult<()> {
    if !path.is_file() {
        return Err(AppError::NotFound(format!(
            "图片不存在: {}",
            path.display()
        )));
    }
    let usable = prepare(path, cache_dir)?;
    match target {
        Target::Home => apply(&usable),
        Target::Lock => apply_lock(&usable),
        Target::Both => {
            // 先后次序不重要，但两个都做，任一失败就报出来 ——
            // 只成功一半的话主人会以为「两个都设了」
            apply(&usable)?;
            apply_lock(&usable)
        }
    }
}

/// 整理成 Windows 认得的格式，返回真正要设的那个路径。
///
/// 绝大多数情况就是原路径直接返回 —— 转码只发生在 WebP / ICO 这类
/// Windows 当面不认的格式上。
fn prepare(path: &Path, cache_dir: &Path) -> AppResult<PathBuf> {
    if is_native(path) {
        return Ok(path.to_path_buf());
    }

    std::fs::create_dir_all(cache_dir)
        .map_err(|err| AppError::Other(format!("创建壁纸缓存目录失败: {err}")))?;
    let target = cache_dir.join(format!("{}.png", cache_key(path)));
    // 同一张图重复设壁纸时复用上次转好的那份，不必每点一次都解码一遍
    if target.is_file() {
        return Ok(target);
    }

    let decoded = image::open(path).map_err(|err| AppError::Other(format!("解码图片失败: {err}")))?;
    decoded
        .save_with_format(&target, image::ImageFormat::Png)
        .map_err(|err| AppError::Other(format!("转码为 PNG 失败: {err}")))?;
    Ok(target)
}

/// 这个文件能不能直接交给 Windows。大小写不敏感 —— 扩展名是用户磁盘上的
/// 原始文件名带来的，`.JPG` 和 `.jpg` 一样常见。
fn is_native(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    NATIVE_EXTENSIONS.contains(&extension.as_str())
}

/// 用路径的哈希当缓存文件名。只求稳定、不要求跨进程一致，所以用标准库的
/// `DefaultHasher` 就够 —— 这里不是持久化标识，只是个缓存键。
fn cache_key(path: &Path) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.to_string_lossy().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(windows)]
fn apply(path: &Path) -> AppResult<()> {
    use std::os::windows::ffi::OsStrExt;

    /// `SPI_SETDESKWALLPAPER`。`pvParam` 要的是以 NUL 结尾的 UTF-16 宽字符串。
    const SPI_SET_DESK_WALLPAPER: u32 = 0x0014;
    /// 把路径写进注册表 —— 少了它，重启后会退回旧壁纸。
    const SPIF_UPDATE_INI_FILE: u32 = 0x01;
    /// 立刻广播 `WM_SETTINGCHANGE` —— 少了它，要等下次登录才换。
    const SPIF_SEND_CHANGE: u32 = 0x02;

    // 只为这一个函数引 windows-sys 不划算（Cargo.lock 里那 40 来个 windows crate
    // 全是别人的传递依赖），直接声明 FFI。签名与 `winuser.h` 一致。
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SystemParametersInfoW(
            ui_action: u32,
            ui_param: u32,
            pv_param: *mut core::ffi::c_void,
            f_win_ini: u32,
        ) -> i32;
    }

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);

    let ok = unsafe {
        SystemParametersInfoW(
            SPI_SET_DESK_WALLPAPER,
            0,
            wide.as_mut_ptr().cast(),
            SPIF_UPDATE_INI_FILE | SPIF_SEND_CHANGE,
        )
    };
    if ok == 0 {
        return Err(AppError::Other(format!(
            "设置壁纸失败（SystemParametersInfoW 返回 0）: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(windows))]
fn apply(_path: &Path) -> AppResult<()> {
    Err(AppError::Other("这个平台还不支持设置桌面壁纸".into()))
}

/// 设**锁屏**背景。
///
/// Win32 里没有对应 API，只有 WinRT 的 `UserProfilePersonalizationSettings` 这一条路。
/// 三个容易踩的点：
///
/// - `IAsyncOperation::get()` 是**阻塞**等待。调用方必须已经在阻塞线程上
///   （这里是 `spawn_blocking` 里），别在主线程上直接等。
/// - WinRT 调用要求当前线程在某个**套间**里。线程池线程默认不在，
///   所以先 `RoInitialize(RO_INIT_MULTITHREADED)`；已经初始化过会返回 `S_FALSE`
///   （不是错误），所以忽略返回值。
/// - 这个接口**不是每个系统版本都可用**，`IsSupported()` 要先问一次；
///   也正因为它会真的换掉用户的锁屏，自动化测试里没有覆盖（见文件末尾说明）。
#[cfg(windows)]
fn apply_lock(path: &Path) -> AppResult<()> {
    use windows::Storage::StorageFile;
    use windows::System::UserProfile::UserProfilePersonalizationSettings;
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    use windows::core::HSTRING;

    // 已经是 MTA 时返回 S_FALSE，不是失败
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }

    let supported = UserProfilePersonalizationSettings::IsSupported()
        .map_err(|err| AppError::Other(format!("查询锁屏壁纸支持情况失败: {err}")))?;
    if !supported {
        return Err(AppError::Other(
            "当前系统不允许程序设置锁屏壁纸（企业策略或系统版本限制）".into(),
        ));
    }

    let settings = UserProfilePersonalizationSettings::Current()
        .map_err(|err| AppError::Other(format!("取锁屏个性化设置失败: {err}")))?;
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_os_str()))
        .and_then(|operation| operation.join())
        .map_err(|err| AppError::Other(format!("打开图片失败: {err}")))?;

    let accepted = settings
        .TrySetLockScreenImageAsync(&file)
        .and_then(|operation| operation.join())
        .map_err(|err| AppError::Other(format!("设置锁屏壁纸失败: {err}")))?;
    if !accepted {
        return Err(AppError::Other("系统拒绝了这次锁屏壁纸设置".into()));
    }
    Ok(())
}

#[cfg(not(windows))]
fn apply_lock(_path: &Path) -> AppResult<()> {
    Err(AppError::Other("这个平台还不支持设置锁屏壁纸".into()))
}

/// 读回当前桌面壁纸的路径。**只给测试用** —— 它存在的意义是：用与 `apply`
/// **同一个 FFI 声明**去调 `SystemParametersInfoW`，能读回一个像样的路径就说明
/// 签名、链接名、宽字符串编码这三点都对；而 `SPI_GET*` 不会改动任何东西，
/// 可以在别人的机器上放心跑。（手写 FFI 最容易错的就是这三点，错了通常直接段错误。）
#[cfg(all(windows, test))]
fn current() -> Option<PathBuf> {
    const SPI_GET_DESK_WALLPAPER: u32 = 0x0073;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SystemParametersInfoW(
            ui_action: u32,
            ui_param: u32,
            pv_param: *mut core::ffi::c_void,
            f_win_ini: u32,
        ) -> i32;
    }

    // 壁纸路径最长可以是 MAX_PATH；给足余量
    let mut buffer = vec![0u16; 4096];
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GET_DESK_WALLPAPER,
            buffer.len() as u32,
            buffer.as_mut_ptr().cast(),
            0,
        )
    };
    if ok == 0 {
        return None;
    }
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(PathBuf::from(String::from_utf16_lossy(&buffer[..end])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_format_table_is_case_insensitive_and_excludes_webp() {
        for name in ["a.jpg", "b.JPEG", "c.png", "d.Bmp", "e.gif", "f.tiff"] {
            assert!(is_native(Path::new(name)), "{name} 是原生格式，不该白白转码");
        }
        for name in ["a.webp", "b.ico", "c.avif", "d", "e."] {
            assert!(!is_native(Path::new(name)), "{name} 必须先转码，否则 Windows 会静默失败");
        }
    }

    #[test]
    fn non_native_format_is_transcoded_and_then_reused() {
        let dir = std::env::temp_dir().join("lc-wallpaper-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 造一张真图，但故意用 Windows 不认的扩展名
        let src = dir.join("壁纸.webp");
        image::RgbImage::from_pixel(4, 4, image::Rgb([200, 30, 30]))
            .save_with_format(&src, image::ImageFormat::WebP)
            .unwrap();

        let cache = dir.join("cache");
        let produced = prepare(&src, &cache).unwrap();
        assert_ne!(produced, src, "非原生格式不能被原样交出去");
        assert_eq!(produced.extension().unwrap(), "png");
        assert!(produced.is_file(), "转码结果必须真的落盘");

        // 再跑一次要复用同一个文件 —— 否则每点一次壁纸都白解码一遍
        assert_eq!(prepare(&src, &cache).unwrap(), produced);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 手写 FFI 的体检：签名、`#[link(name = "user32")]`、宽字符串编码，
    /// 三点里错任何一点这里就会读回空路径甚至直接段错误。
    #[cfg(windows)]
    #[test]
    fn user32_ffi_declaration_is_correct() {
        let current = current().expect("SPI_GETDESKWALLPAPER 应该能读到当前壁纸");
        assert!(
            current.is_absolute(),
            "读回来的应该是个绝对路径，实际是 {}",
            current.display()
        );
    }

    /// 真的设一次壁纸。**会改动桌面**，所以默认不跑（`#[ignore]`），
    /// 手动验证时用 `cargo test -- --ignored set_wallpaper_end_to_end`
    /// —— 而且记得先把原壁纸存下来再还原（见 README 的验证步骤）。
    #[cfg(windows)]
    #[ignore]
    #[test]
    fn set_wallpaper_end_to_end() {
        let dir = std::env::temp_dir().join("lc-wallpaper-e2e");
        std::fs::create_dir_all(&dir).unwrap();
        let image_path = dir.join("probe.png");
        image::RgbImage::from_pixel(64, 64, image::Rgb([12, 200, 240]))
            .save(&image_path)
            .unwrap();

        set(&image_path, &dir.join("cache"), Target::Home).expect("设置壁纸应当成功");

        // 读回来验证：Windows 会把源图转码后落到 TranscodedWallpaper，
        // 所以这里比对的是「注册表里的源路径」而不是像素
        let now = current().expect("设完之后应该能读回壁纸路径");
        assert!(
            now.to_string_lossy().contains("lc-wallpaper-e2e"),
            "设完读回的应该是刚才那张，实际是 {}",
            now.display()
        );
    }

    /// 锁屏那条路的**只读**体检：WinRT 套间初始化、类型激活、crate 管道对不对。
    ///
    /// 刻意只调 `IsSupported()`，**不真的设一次** —— 设一次就会换掉用户的锁屏图，
    /// 而 Windows 没有「读回当前锁屏图路径」的正规接口，换掉了还原不回去。
    /// 这条测试挡得住最可能出问题的地方（线程没进 MTA 套间、激活失败）。
    #[cfg(windows)]
    #[test]
    fn winrt_lock_screen_plumbing_is_alive() {
        use windows::System::UserProfile::UserProfilePersonalizationSettings;
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        let supported = UserProfilePersonalizationSettings::IsSupported()
            .expect("WinRT 激活应当成功（失败通常意味着本线程没进 MTA 套间）");
        println!("本机是否允许程序设锁屏壁纸: {supported}");
    }

    #[test]
    fn cache_key_is_stable_and_path_specific() {
        let a = cache_key(Path::new("C:/a/1.jpg"));
        let b = cache_key(Path::new("C:/a/1.jpg"));
        let c = cache_key(Path::new("C:/a/2.jpg"));
        assert_eq!(a, b, "同一个路径必须得到同一个键，否则缓存形同虚设");
        assert_ne!(a, c, "不同路径不能撞键，否则会拿错图");
    }
}

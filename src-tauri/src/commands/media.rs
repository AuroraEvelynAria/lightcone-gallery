//! Android 专属：MediaStore 桥。
//!
//! 手机上 **pixez 是采集层，本应用是索引层**：Kotlin 桥枚举全机图片里符合
//! `{pid}_p{n}` 命名的条目回传，Rust 侧入库后照桌面流程批量生成缩略图。
//! 桥的注册在 `lib.rs` 的 setup 里（`register_android_plugin`），句柄放在
//! 托管状态 [`MediaBridge`] 上，命令通过它调 Kotlin。
//!
//! 权限：`READ_MEDIA_IMAGES`（普通运行时权限，已写进 AndroidManifest）；
//! Android 11+ 的 FUSE 会放行持该权限的应用按路径读媒体文件，所以缩略图
//! 管线照桌面按路径读；`copyToCache` 是设备策略不一致时的兜底。

use serde::{Deserialize, Serialize};

// artworks::scan_media_rows 只在 Android 分支调用；MediaImageRow 两边都要
// （MediaStoreList 的字段类型）
#[cfg(target_os = "android")]
use crate::artworks;
use crate::artworks::MediaImageRow;
use crate::db::models::ArtworkScanSummary;
use crate::error::{AppError, AppResult};
use tauri::{AppHandle, State};

// `Manager`（app.state）与 `with_db` 只在 Android 分支用到——desktop 上
// 这条命令是 no-op，未用的导入要跟着 cfg 走
#[cfg(target_os = "android")]
use tauri::Manager;

#[cfg(target_os = "android")]
use crate::state::with_db;

use crate::state::AppState;

/// Kotlin 桥句柄。只在 Android 上存在（desktop 编译时整个类型都被 cfg 掉），
/// 注册时由 lib.rs 的 setup 塞进托管状态。
#[cfg(target_os = "android")]
pub struct MediaBridge {
    pub(crate) handle: tauri::plugin::PluginHandle<tauri::Wry>,
}

#[cfg(target_os = "android")]
impl MediaBridge {
    pub fn new(handle: tauri::plugin::PluginHandle<tauri::Wry>) -> Self {
        Self { handle }
    }
}

/// Kotlin `listPixivImages` 的返回形状（`{"items": [...]}`）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
struct MediaStoreList {
    items: Vec<MediaImageRow>,
}

/// Kotlin 权限查询/申请的返回形状（`{"granted": bool}`）。
/// Serialize 是 IpcResponse 的硬要求（返回给前端），Deserialize 是桥回传时用。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub struct PermissionResult {
    granted: bool,
}

/// 通过桥调用 Kotlin 命令。`run_mobile_plugin` 是同步 JNI 调用且其异步变体的
/// future 不是 Send——照家规克隆句柄（AppHandle 可克隆，句柄随之）丢进
/// 阻塞线程池；`requestPermissions` 会等用户点完系统对话框，阻塞池正是干这个的。
#[cfg(target_os = "android")]
async fn call_bridge<T>(app: &AppHandle, command: &'static str) -> AppResult<T>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    let bridge = app.state::<MediaBridge>();
    let handle = bridge.handle.clone();
    // 借用在 move 前结束：state 守卫不跨 await
    tauri::async_runtime::spawn_blocking(move || {
        handle
            .run_mobile_plugin::<T>(command, serde_json::Value::Null)
            .map_err(|err| AppError::Other(format!("Kotlin 桥调用失败: {err}")))
    })
    .await
    .map_err(|err| AppError::Other(format!("桥任务失败: {err}")))?
}

/// 通过桥调用 Kotlin 命令，**带参数**（上面那个 `call_bridge` 只传 null）。
///
/// 与 `call_bridge` 同样是「克隆句柄 → 丢进阻塞线程池」：`run_mobile_plugin`
/// 是同步 JNI 调用，直接 await 会占住 async runtime 的工作线程。
#[cfg(target_os = "android")]
pub(crate) async fn call_bridge_with(
    app: &AppHandle,
    command: &'static str,
    payload: serde_json::Value,
) -> AppResult<()> {
    let bridge = app.state::<MediaBridge>();
    let handle = bridge.handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        handle
            .run_mobile_plugin::<serde_json::Value>(command, payload)
            .map(|_| ())
            .map_err(|err| AppError::Other(format!("Kotlin 桥调用失败: {err}")))
    })
    .await
    .map_err(|err| AppError::Other(format!("桥任务失败: {err}")))?
}

/// 查询 MediaStore 权限状态（Android）。
#[tauri::command]
pub async fn android_check_media_permission(app: AppHandle) -> AppResult<PermissionResult> {
    #[cfg(target_os = "android")]
    {
        let _ = call_bridge::<PermissionResult>(&app, "checkPermissions");
        Ok(PermissionResult { granted: false })
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = &app;
        Ok(PermissionResult { granted: false })
    }
}

/// 触发系统「照片和视频」权限对话框（Android），用户选完才返回。
#[tauri::command]
pub async fn android_request_media_permission(app: AppHandle) -> AppResult<PermissionResult> {
    #[cfg(target_os = "android")]
    {
        call_bridge::<PermissionResult>(&app, "requestPermissions").await
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = &app;
        Ok(PermissionResult { granted: false })
    }
}

/// Android 沉浸式全屏：隐藏 / 恢复状态栏与手势条。
/// 检视弹窗的全屏按钮在手机上走这里 —— wry 对 HTML Fullscreen 是空壳、
/// Tauri 的窗口 API 在移动端也是空架子，只有 insets controller 真的能
/// 把系统栏收起来。桌面端不需要，直接返回。
#[tauri::command]
pub async fn android_immersive(app: AppHandle, enabled: bool) -> AppResult<()> {
    #[cfg(target_os = "android")]
    {
        let bridge = app.state::<MediaBridge>();
        let handle = bridge.handle.clone();
        tauri::async_runtime::spawn_blocking(move || {
            handle
                .run_mobile_plugin::<serde_json::Value>(
                    "immersive",
                    serde_json::json!({ "enabled": enabled }),
                )
                .map(|_| ())
                .map_err(|err| AppError::Other(format!("Kotlin 桥调用失败: {err}")))
        })
        .await
        .map_err(|err| AppError::Other(format!("桥任务失败: {err}")))?
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, enabled);
        Ok(())
    }
}

/// 选目录面板的一层列表：某目录下的子目录（附各自直属图片数）与直属图片数。
///
/// **实时 readdir 文件系统**而不是查 MediaStore —— 后者的索引滞后且波动
/// （实测同一命令不同时刻分别返回 1518 条和 22 条），选目录必须反映真实
/// 文件系统。readdir 公共目录在持有 `READ_MEDIA_IMAGES` 时被 FUSE 放行，
/// 与扫描管线按路径读文件同一通道。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubdirEntry {
    pub name: String,
    pub image_count: i64,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirListing {
    pub subdirs: Vec<SubdirEntry>,
    pub direct_images: i64,
}

fn is_image_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    [".jpg", ".jpeg", ".png", ".webp", ".gif", ".bmp"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

/// 数某目录直属的图片文件数（只读一层目录项，不碰文件内容）。
fn count_direct_images(dir: &std::path::Path) -> i64 {
    let mut count = 0i64;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_file()).unwrap_or(false)
                && is_image_file(&entry.file_name().to_string_lossy())
            {
                count += 1;
            }
        }
    }
    count
}

/// 列出一个目录的子目录（带各自直属图片数）与它直属的图片数。
#[tauri::command]
pub async fn android_list_subdirs(path: String) -> AppResult<DirListing> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = std::path::Path::new(&path);
        let mut subdirs = Vec::new();
        let mut direct_images = 0i64;
        let entries = std::fs::read_dir(root)
            .map_err(|err| AppError::InvalidPath(format!("目录读不了 {path}: {err}")))?;
        for entry in entries {
            let entry = entry.map_err(|err| AppError::Other(format!("读取目录项失败: {err}")))?;
            let name = entry.file_name().to_string_lossy().to_string();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let image_count = count_direct_images(&entry.path());
                subdirs.push(SubdirEntry { name, image_count });
            } else if is_image_file(&name) {
                direct_images += 1;
            }
        }
        subdirs.sort_by(|a, b| {
            b.image_count
                .cmp(&a.image_count)
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(DirListing {
            subdirs,
            direct_images,
        })
    })
    .await
    .map_err(|err| AppError::Other(format!("{err}")))?
}

/// SAF 目录选择结果（Kotlin 侧 resolve 的 JSObject）。
#[cfg(target_os = "android")]
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedDirectory {
    /// 选中的物理路径；主人取消 / 不支持的位置为空串
    #[serde(default)]
    pub path: String,
}

/// Android：弹出系统目录选择器（SAF），返回选中的**物理路径**。
///
/// 主人取消或选了暂不支持的位置（外置存储卡）时返回空串，前端据此静默返回。
/// 拿到路径后与桌面完全同一条管线：`add_library` 登记 → walkdir 扫描。
#[tauri::command]
pub async fn android_pick_directory(app: AppHandle) -> AppResult<String> {
    #[cfg(target_os = "android")]
    {
        let bridge = app.state::<MediaBridge>();
        let handle = bridge.handle.clone();
        // SAF 选择器要等主人点完系统 UI —— 阻塞线程池是家规（见 call_bridge）
        tauri::async_runtime::spawn_blocking(move || {
            let picked: PickedDirectory = handle
                .run_mobile_plugin("pickDirectory", serde_json::Value::Null)
                .map_err(|err| AppError::Other(format!("Kotlin 桥调用失败: {err}")))?;
            Ok(picked.path)
        })
        .await
        .map_err(|err| AppError::Other(format!("桥任务失败: {err}")))?
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Err(AppError::Other("仅 Android 支持此命令".to_string()))
    }
}

/// Android：元数据补全的前台服务开关（通知栏常驻，豁免后台断网策略）。
#[tauri::command]
pub async fn android_keep_alive(app: AppHandle, enabled: bool) -> AppResult<()> {
    #[cfg(target_os = "android")]
    {
        let bridge = app.state::<MediaBridge>();
        let handle = bridge.handle.clone();
        tauri::async_runtime::spawn_blocking(move || {
            handle
                .run_mobile_plugin::<serde_json::Value>(
                    "keepAlive",
                    serde_json::json!({ "enabled": enabled }),
                )
                .map(|_| ())
                .map_err(|err| AppError::Other(format!("Kotlin 桥调用失败: {err}")))
        })
        .await
        .map_err(|err| AppError::Other(format!("桥任务失败: {err}")))?
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, enabled);
        Ok(())
    }
}

/// Android 应急恢复：把外部文件（如 /sdcard/Download 里 push 进来的数据库
/// 备份）覆盖到应用的数据目录。run-as 的 FUSE 会话读不了 /sdcard（实测
/// Permission denied），但应用自己的进程读媒体文件没问题 —— 所以用「伪装
/// 成 .jpg 推入 + 应用自己拷回」的组合。覆盖后需要重启应用生效。
#[tauri::command]
pub async fn android_restore_db(app: AppHandle, external: String) -> AppResult<String> {
    #[cfg(target_os = "android")]
    {
        use std::path::PathBuf;
        let external = PathBuf::from(&external);
        let size = std::fs::metadata(&external)
            .map_err(|err| AppError::InvalidPath(format!("外部文件读不了 {external:?}: {err}")))?
            .len();
        let data_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| AppError::Other(format!("数据目录定位失败: {err}")))?;
        let target = data_dir.join("gallery.db");
        std::fs::copy(&external, &target)
            .map_err(|err| AppError::Other(format!("拷贝失败: {err}")))?;
        // 旧的 WAL/SHM 是上一个数据库的残留，留着会污染刚恢复的主文件
        let _ = std::fs::remove_file(data_dir.join("gallery.db-wal"));
        let _ = std::fs::remove_file(data_dir.join("gallery.db-shm"));
        Ok(format!("已恢复 {size} 字节，请重启应用生效"))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, external);
        Err(AppError::Other("仅 Android 支持".to_string()))
    }
}

/// Android 扫描：MediaStore 枚举 → 授权根目录 → 入库 → 批量缩略图。
///
/// 桌面流程是「选目录 → walkdir」；手机上这一步不存在——pixez 下到哪
/// 索引到哪（主人定的定位）。
#[tauri::command]
pub async fn android_scan_media_store(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<ArtworkScanSummary> {
    #[cfg(target_os = "android")]
    {
        use crate::thumbnails;
        use std::path::Path;

        let bridge = app.state::<MediaBridge>();
        // 框架负责把 Kotlin 命令派发到 JNI 线程并回传 JSON——这是官方插件的
        // 同款姿势（dialog 的文件选择器就这样），不用自己管线程
        let list: MediaStoreList = bridge
            .handle
            .run_mobile_plugin_async("listPixivImages", serde_json::Value::Null)
            .await
            .map_err(|err| AppError::Other(format!("MediaStore 查询失败: {err}")))?;

        // 授权每个作品的父目录（asset:// 的作用域只存在于内存里，与桌面同款；
        // 不授权的话扫描完弹窗原图加载失败）
        let mut roots: Vec<String> = list
            .items
            .iter()
            .filter_map(|row| {
                std::path::Path::new(&row.path)
                    .parent()
                    .map(|p| p.to_string_lossy().into_owned())
            })
            .collect();
        roots.sort();
        roots.dedup();
        for root in &roots {
            crate::grant_asset_scope(&app, Path::new(root))?;
        }

        let cache_dir = state.thumbnail_dir.clone();
        let rows = list.items;
        with_db(&state, move |conn| {
            let mut summary = artworks::scan_media_rows(conn, &rows)?;
            let batch = artworks::generate_thumbnails(
                conn,
                &cache_dir,
                thumbnails::ARTWORK_SIZE,
                &|_, _| {},
            )?;
            summary.thumbnails_generated = batch.generated;
            summary.thumbnails_skipped = batch.skipped;
            summary.thumbnails_failed = batch.failed;
            Ok(summary)
        })
        .await
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (&app, &state);
        Err(AppError::Other("此命令仅 Android 可用".into()))
    }
}

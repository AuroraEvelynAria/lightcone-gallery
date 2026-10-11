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

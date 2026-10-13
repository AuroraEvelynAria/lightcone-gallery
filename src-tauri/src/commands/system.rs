//! 全局统计、缓存维护、窗口材质与键值设置命令。

use std::collections::HashMap;

use rusqlite::params;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::db::models::Stats;
use crate::error::{AppError, AppResult};
use crate::state::{AppState, with_db};
use crate::thumbnails;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheUsage {
    pub bytes: u64,
    pub files: u64,
}

#[tauri::command]
pub async fn library_stats(state: State<'_, AppState>) -> AppResult<Stats> {
    with_db(&state, |conn| {
        let scalar = |sql: &str| -> AppResult<i64> { Ok(conn.query_row(sql, [], |row| row.get(0))?) };

        Ok(Stats {
            images: scalar("SELECT COUNT(*) FROM images")?,
            libraries: scalar("SELECT COUNT(*) FROM libraries")?,
            albums: scalar("SELECT COUNT(*) FROM albums")?,
            tags: scalar("SELECT COUNT(*) FROM tags")?,
            favorites: scalar("SELECT COUNT(*) FROM images WHERE is_favorite = 1")?,
            total_bytes: scalar("SELECT COALESCE(SUM(file_size), 0) FROM images")?,
        })
    })
    .await
}

#[tauri::command]
pub async fn thumbnail_cache_usage(state: State<'_, AppState>) -> AppResult<CacheUsage> {
    let cache_dir = state.thumbnail_dir.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let (bytes, files) = thumbnails::cache_usage(&cache_dir);
        Ok(CacheUsage { bytes, files })
    })
    .await
    .map_err(|err| crate::error::AppError::Other(format!("统计缓存失败: {err}")))?
}

#[tauri::command]
pub async fn clear_thumbnail_cache(state: State<'_, AppState>) -> AppResult<()> {
    let cache_dir = state.thumbnail_dir.clone();

    tauri::async_runtime::spawn_blocking(move || thumbnails::clear_cache(&cache_dir))
        .await
        .map_err(|err| AppError::Other(format!("清理缓存失败: {err}")))?
}

/// 应用窗口材质（云母 / 亚克力 / 标签页 / 模糊），`solid` 表示撤掉材质回到纯色。
///
/// `dark` 用来选材质的明暗变体：只有 Mica / Tabbed 提供了显式的明暗变体，
/// 若只传 `Effect::Mica` 它会跟随**系统**偏好，跟应用自己的明暗设置对不上。
///
/// `tint` 是材质染色（`#rrggbb`），**只有 Acrylic / Blur 吃这个参数**。
/// 这两个材质本身几乎不挡壁纸，实测在浅色壁纸下会把深色方案冲淡到读不清字，
/// 所以要用配色方案自己的底色染一遍。
///
/// **平台限制**：Mica / Tabbed 需要 Windows 11（build ≥ 22000），Acrylic 需要
/// Windows 10 v1809+，Blur 在 Windows 11 22621 上拖动窗口会明显卡顿。
///
/// **注意**：Tauri 的 `apply_effects` 把底层 `window_vibrancy` 的错误整个丢掉了
/// （见 `tauri/src/vibrancy/windows.rs`），所以本命令**无法**告知材质是否真的生效——
/// 在不支持的系统上它会静默无操作。前端的做法是在选项旁标注平台要求，不假装能检测。
#[tauri::command]
pub async fn set_window_material(
    window: tauri::WebviewWindow,
    material: String,
    dark: bool,
    tint: Option<String>,
) -> AppResult<()> {
    use tauri::utils::config::WindowEffectsConfig;
    use tauri::window::{Color, Effect, EffectsBuilder};

    let effect = match material.as_str() {
        "solid" => None,
        "mica" => Some(if dark { Effect::MicaDark } else { Effect::MicaLight }),
        "acrylic" => Some(Effect::Acrylic),
        "tabbed" => Some(if dark {
            Effect::TabbedDark
        } else {
            Effect::TabbedLight
        }),
        "blur" => Some(Effect::Blur),
        other => {
            return Err(AppError::InvalidInput(format!("未知的窗口材质: {other}")));
        }
    };

    let config: Option<WindowEffectsConfig> = match effect {
        None => None,
        Some(effect) => {
            let mut builder = EffectsBuilder::new().effect(effect);
            if matches!(effect, Effect::Acrylic | Effect::Blur)
                && let Some(tint) = tint.as_deref()
            {
                builder = builder.color(
                    tint.parse::<Color>()
                        .map_err(|err| AppError::InvalidInput(format!("材质染色格式不对: {err}")))?,
                );
            }
            Some(builder.build())
        }
    };

    // 材质是 Windows DWM 的专有能力，`WebviewWindow::set_effects` 只在桌面平台
    // 编译（mobile 上没有这个方法，交叉编译在这里挂过）。移动端验证过参数就
    // 吞掉——前端把材质选项藏起来之前，这条 no-op 保证命令不报错。
    #[cfg(desktop)]
    window
        .set_effects(config)
        .map_err(|err| AppError::Other(format!("应用窗口材质失败: {err}")))?;
    #[cfg(not(desktop))]
    {
        let _ = config;
        let _ = &window;
    }
    Ok(())
}

/// 读取全部键值设置。
///
/// 主题偏好这类「界面状态」放数据库而不是 WebView 的 localStorage —— 后者会被
/// 清缓存顺手清掉，用户的外观设置不该跟着一起消失。
#[tauri::command]
pub async fn load_settings(state: State<'_, AppState>) -> AppResult<HashMap<String, String>> {
    with_db(&state, |conn| {
        let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<HashMap<_, _>>>()?)
    })
    .await
}

/// 写入若干键值设置（同名的键直接覆盖）。
#[tauri::command]
pub async fn save_settings(
    state: State<'_, AppState>,
    entries: HashMap<String, String>,
) -> AppResult<()> {
    with_db(&state, move |conn| {
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )?;
            for (key, value) in &entries {
                stmt.execute(params![key, value])?;
            }
        }
        tx.commit()?;
        Ok(())
    })
    .await
}

/// 把一张原图写进系统剪贴板。
///
/// ## 为什么需要它
///
/// WebView2 右键菜单里的「复制图片」在本应用里点了没反应 —— 它要靠图片的 URL
/// 去取字节，而我们的图走的是自定义 `asset://` 协议，它取不到。所以自己来：
/// 从 Rust 直接读文件、解码、写剪贴板，不依赖 WebView2 怎么实现。
///
/// 解码放在阻塞线程池上：一张 4000×6000 的图解码要一两秒，占住 async runtime
/// 会卡住别的命令。
#[tauri::command]
pub async fn copy_image_to_clipboard(app: AppHandle, path: String) -> AppResult<()> {
    let (rgba, width, height) = tauri::async_runtime::spawn_blocking(move || {
        let decoded = image::open(&path)?;
        let rgba = decoded.to_rgba8();
        let (width, height) = rgba.dimensions();
        Ok::<_, AppError>((rgba.into_raw(), width, height))
    })
    .await
    .map_err(|err| AppError::Other(format!("解码任务失败: {err}")))??;

    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard()
        .write_image(&tauri::image::Image::new_owned(rgba, width, height))
        .map_err(|err| AppError::Other(format!("写入剪贴板失败: {err}")))?;
    Ok(())
}

/// 把一张图设为**桌面壁纸**（双端）。
///
/// 两端实现完全不同：Windows 走 `SystemParametersInfoW`（见 `crate::wallpaper`），
/// Android 交给 Kotlin 的 `WallpaperManager`（见 `MediaStoreBridge` 的 `setWallpaper`）。
///
/// `target` **只对 Android 有意义**：`home`（默认，桌面）/ `lock`（锁屏）/ `both`。
/// Windows 没有「锁屏壁纸」这个概念 —— 那是登录界面的背景，普通程序改不了，
/// 所以传了也忽略。
///
/// 解码 / 转码是 CPU 密集的（一张 4000×6000 的图要一两秒），丢到阻塞线程池上，
/// 与 `copy_image_to_clipboard` 同一个理由：占住 async runtime 会卡住别的命令。
#[tauri::command]
pub async fn set_wallpaper(app: AppHandle, path: String, target: Option<String>) -> AppResult<()> {
    #[cfg(target_os = "android")]
    {
        let payload = serde_json::json!({
            "path": path,
            // 默认只设桌面：锁屏壁纸在部分 ROM 上不支持，会白弹一次失败
            "target": target.unwrap_or_else(|| "home".to_string()),
        });
        crate::commands::media::call_bridge_with(&app, "setWallpaper", payload).await
    }

    #[cfg(not(target_os = "android"))]
    {
        let wanted = crate::wallpaper::Target::parse(target.as_deref());
        // 转码后的 PNG 落在应用数据目录而不是临时目录：注册表里存的是这个路径，
        // 重启之后还得读得到
        let cache_dir = app
            .path()
            .app_data_dir()
            .map_err(|err| AppError::Other(format!("取应用数据目录失败: {err}")))?
            .join("wallpaper");
        tauri::async_runtime::spawn_blocking(move || {
            crate::wallpaper::set(std::path::Path::new(&path), &cache_dir, wanted)
        })
        .await
        .map_err(|err| AppError::Other(format!("壁纸任务失败: {err}")))?
    }
}

/// Android 全面屏返回手势的最后一站：MainActivity 把系统返回转发给前端，
/// 前端的「关闭栈」（弹窗 → 抽屉 → …）都空了才调这里退出应用。
/// `app.exit` 会走 `RunEvent::Exit`，lib.rs 里的 WAL checkpoint 照常执行。
#[tauri::command]
pub fn exit_app(app: AppHandle) {
    app.exit(0);
}

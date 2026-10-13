//! LightCone Gallery 后端入口。
//!
//! 启动流程：定位 app data 目录 -> 打开并迁移 SQLite -> 恢复 asset:// 目录授权 -> 注册命令。

mod artists;
mod artworks;
mod commands;
mod db;
mod error;
mod pixiv;
mod scanner;
mod state;
mod thumbnails;
mod wallpaper;

use std::path::Path;

use tauri::Manager;

use crate::commands::grant_asset_scope;
use crate::state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        // 剪贴板插件：只用它的 Rust API（`app.clipboard().write_image`），
        // 走的是进程内调用、不经过 IPC，所以不需要在 capabilities 里放行它的命令。
        .plugin(tauri_plugin_clipboard_manager::init())
        // MediaStore 桥只在 Android 存在：注册后把手柄交给托管状态，
        // `android_scan_media_store` 命令通过它调 Kotlin（见 commands/media.rs）
        .plugin(
            tauri::plugin::Builder::<tauri::Wry, serde_json::Value>::new("media-store-bridge")
                .setup(|_app, _api| {
                    #[cfg(target_os = "android")]
                    {
                        use commands::media::MediaBridge;
                        let handle = _api
                            .register_android_plugin("com.lightcone.gallery.media", "MediaStoreBridge")?;
                        _app.manage(MediaBridge::new(handle));
                    }
                    Ok(())
                })
                .build(),
        )
        .setup(|app| {
            // Android：初始化 rustls-platform-verifier —— reqwest 0.13 的默认 TLS
            // 在 Android 上靠它拿系统证书，而它的初始化要 JNI 环境（官方要求应用
            // 启动时手动调）。漏掉的话每次 HTTPS 连接直接 panic
            // 「Expect rustls-platform-verifier to be initialized」，任务悄悄死掉。
            #[cfg(target_os = "android")]
            {
                use jni::objects::JObject;
                let ctx = ndk_context::android_context();
                let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) };
                vm.attach_current_thread(|env| {
                    let context =
                        unsafe { JObject::from_raw(env, ctx.context().cast()) };
                    rustls_platform_verifier::android::init_with_env(env, context)
                })
                .map_err(|err| format!("TLS verifier 初始化失败: {err}"))?;
            }

            // AppHandle 是 clone 成本极低的句柄，先取出来避免与 app.manage 的可变借用冲突
            let handle = app.handle().clone();

            let data_dir = handle.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;

            let conn = db::open(&data_dir.join("gallery.db"))?;
            let thumbnail_dir = data_dir.join("thumbnails");
            std::fs::create_dir_all(&thumbnail_dir)?;

            // asset:// 的 scope 只存在于内存中，每次启动都要重新登记：
            // 1) 缩略图缓存目录；2) 已收录的图库根目录；3) 作品库的扫描根目录（弹窗要显示原图）
            grant_asset_scope(&handle, &thumbnail_dir)?;

            let roots: Vec<String> = {
                let mut stmt = conn.prepare(
                    "SELECT path FROM libraries
                     UNION
                     SELECT path FROM artwork_roots",
                )?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            for root in roots {
                if let Err(err) = grant_asset_scope(&handle, Path::new(&root)) {
                    // 目录可能已被移动或卸载，记录后继续启动
                    eprintln!("恢复本地资源授权失败 {root}: {err}");
                }
            }

            app.manage(AppState::new(conn, thumbnail_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // 图库
            commands::list_libraries,
            commands::add_library,
            commands::remove_library,
            commands::scan_library,
            commands::scan_all_libraries,
            // 图片
            commands::list_images,
            commands::get_image,
            commands::toggle_favorite,
            commands::set_rating,
            commands::thumbnail_for,
            // 相册
            commands::list_albums,
            commands::create_album,
            commands::rename_album,
            commands::delete_album,
            commands::add_images_to_album,
            commands::remove_images_from_album,
            commands::set_album_cover,
            // 标签
            commands::list_tags,
            commands::create_tag,
            commands::update_tag,
            commands::delete_tag,
            commands::add_tag_to_image,
            commands::remove_tag_from_image,
            // 系统
            commands::library_stats,
            commands::thumbnail_cache_usage,
            commands::clear_thumbnail_cache,
            // 阶段三：外观（窗口材质 + 键值设置）
            commands::set_window_material,
            commands::load_settings,
            commands::save_settings,
            // 阶段一：作品（Pixiv）维度
            commands::scan_artworks_folder,
            commands::list_artworks,
    commands::list_artwork_aspects,
            // Android：MediaStore 桥（pixez 下图、本应用索引）
            commands::android_scan_media_store,
            commands::android_check_media_permission,
            commands::android_request_media_permission,
            commands::android_immersive,
            commands::android_pick_directory,
            commands::android_list_subdirs,
            commands::android_keep_alive,
            commands::android_restore_db,
            // 阶段二：作品缩略图（有损 WebP）+ 前端虚拟滚动
            commands::thumbnail_for_artwork,
            // 阶段四：把作品移出作品库（不动原图）
            commands::remove_artworks,
            commands::count_ignored_artworks,
            commands::clear_ignored_artworks,
            commands::list_artists,
            commands::count_missing_meta,
            commands::fetch_missing_meta,
            commands::list_pixiv_tags,
            commands::pixiv_meta_stats,
            commands::illust_detail,
            commands::reset_missing_meta,
            commands::fetch_all_meta,
            commands::stop_meta_fetch,
            commands::copy_image_to_clipboard,
            // 设为桌面壁纸（Windows 走 user32 FFI，Android 走 Kotlin 的 WallpaperManager）
            commands::set_wallpaper,
            // 标签图库：侧栏里「一组标签 + 一个分级」的独立图库
            // （内置的「全年龄 / R-18」两条不落库，只有自建的走这三个命令）
            commands::list_tag_galleries,
            commands::create_tag_gallery,
            commands::delete_tag_gallery,
            commands::count_artworks_by_rating,
            // Android：返回手势的最后一站（MainActivity 转发、前端关闭栈走完才调）
            commands::exit_app,
        ])        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|handle, event| {
        // Tauri 的退出路径会直接结束进程、跳过析构，`Connection` 不会走 Drop，
        // 于是 WAL 永远合并不回主库。这里在退出前显式 checkpoint 一次。
        if let tauri::RunEvent::Exit = event
            && let Some(state) = handle.try_state::<AppState>()
            && let Ok(conn) = state.db.lock()
        {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }
    });
}

//! LightCone Gallery 后端入口。
//!
//! 启动流程：定位 app data 目录 -> 打开并迁移 SQLite -> 恢复 asset:// 目录授权 -> 注册命令。

mod artworks;
mod commands;
mod db;
mod error;
mod scanner;
mod state;
mod thumbnails;

use std::path::Path;

use tauri::Manager;

use crate::commands::grant_asset_scope;
use crate::state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
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
            // 阶段二：作品缩略图（有损 WebP）+ 前端虚拟滚动
            commands::thumbnail_for_artwork,
            // 阶段四：把作品移出作品库（不动原图）
            commands::remove_artworks,
            commands::count_ignored_artworks,
            commands::clear_ignored_artworks,
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

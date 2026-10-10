//! 相册相关命令。

use rusqlite::{Connection, OptionalExtension, params};
use tauri::State;

use crate::db::models::{Album, validate_name};
use crate::db::{now_unix, is_unique_violation};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, with_db};

const ALBUM_SELECT: &str = "
    SELECT a.id,
           a.name,
           a.description,
           a.cover_image_id,
           a.created_at,
           (SELECT COUNT(*) FROM album_images ai WHERE ai.album_id = a.id) AS image_count
    FROM albums a";

fn fetch_album(conn: &Connection, id: i64) -> AppResult<Album> {
    conn.query_row(&format!("{ALBUM_SELECT} WHERE a.id = ?1"), [id], Album::from_row)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("相册 {id} 不存在")))
}

#[tauri::command]
pub async fn list_albums(state: State<'_, AppState>) -> AppResult<Vec<Album>> {
    with_db(&state, |conn| {
        let mut stmt = conn.prepare(&format!("{ALBUM_SELECT} ORDER BY a.name COLLATE NOCASE"))?;
        let rows = stmt.query_map([], Album::from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await
}

#[tauri::command]
pub async fn create_album(
    state: State<'_, AppState>,
    name: String,
    description: Option<String>,
) -> AppResult<Album> {
    let name = validate_name("相册名", &name)?;

    with_db(&state, move |conn| {
        conn.execute(
            "INSERT INTO albums (name, description, created_at) VALUES (?1, ?2, ?3)",
            params![name, description, now_unix()],
        )
        .map_err(|err| {
            if is_unique_violation(&err) {
                AppError::InvalidInput(format!("相册「{name}」已存在"))
            } else {
                AppError::Database(err)
            }
        })?;
        fetch_album(conn, conn.last_insert_rowid())
    })
    .await
}

#[tauri::command]
pub async fn rename_album(state: State<'_, AppState>, id: i64, name: String) -> AppResult<Album> {
    let name = validate_name("相册名", &name)?;

    with_db(&state, move |conn| {
        let affected = conn
            .execute("UPDATE albums SET name = ?1 WHERE id = ?2", params![name, id])
            .map_err(|err| {
                if is_unique_violation(&err) {
                    AppError::InvalidInput(format!("相册「{name}」已存在"))
                } else {
                    AppError::Database(err)
                }
            })?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("相册 {id} 不存在")));
        }
        fetch_album(conn, id)
    })
    .await
}

#[tauri::command]
pub async fn delete_album(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    with_db(&state, move |conn| {
        // album_images 由外键级联清理；图片自身不受影响
        let affected = conn.execute("DELETE FROM albums WHERE id = ?1", [id])?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("相册 {id} 不存在")));
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn add_images_to_album(
    state: State<'_, AppState>,
    album_id: i64,
    image_ids: Vec<i64>,
) -> AppResult<i64> {
    if image_ids.is_empty() {
        return Ok(0);
    }

    with_db(&state, move |conn| {
        let now = now_unix();
        let tx = conn.transaction()?;
        let mut added = 0i64;
        {
            // position 追加到末尾，保持用户加入顺序
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO album_images (album_id, image_id, position, added_at)
                 VALUES (
                     ?1, ?2,
                     COALESCE((SELECT MAX(position) + 1 FROM album_images WHERE album_id = ?1), 0),
                     ?3
                 )",
            )?;
            for image_id in &image_ids {
                added += stmt.execute(params![album_id, image_id, now])? as i64;
            }
        }
        tx.commit()?;
        Ok(added)
    })
    .await
}

#[tauri::command]
pub async fn remove_images_from_album(
    state: State<'_, AppState>,
    album_id: i64,
    image_ids: Vec<i64>,
) -> AppResult<i64> {
    if image_ids.is_empty() {
        return Ok(0);
    }

    with_db(&state, move |conn| {
        let tx = conn.transaction()?;
        let mut removed = 0i64;
        {
            let mut stmt =
                tx.prepare("DELETE FROM album_images WHERE album_id = ?1 AND image_id = ?2")?;
            for image_id in &image_ids {
                removed += stmt.execute(params![album_id, image_id])? as i64;
            }
        }
        // 封面被移除时清空，避免指向不属于该相册的图片
        tx.execute(
            "UPDATE albums SET cover_image_id = NULL
             WHERE id = ?1
               AND cover_image_id IS NOT NULL
               AND cover_image_id NOT IN (SELECT image_id FROM album_images WHERE album_id = ?1)",
            [album_id],
        )?;
        tx.commit()?;
        Ok(removed)
    })
    .await
}

#[tauri::command]
pub async fn set_album_cover(
    state: State<'_, AppState>,
    album_id: i64,
    image_id: Option<i64>,
) -> AppResult<()> {
    with_db(&state, move |conn| {
        if let Some(image_id) = image_id {
            let belongs: i64 = conn.query_row(
                "SELECT COUNT(*) FROM album_images WHERE album_id = ?1 AND image_id = ?2",
                params![album_id, image_id],
                |row| row.get(0),
            )?;
            if belongs == 0 {
                return Err(AppError::InvalidInput("该图片不在相册中，无法设为封面".into()));
            }
        }

        let affected = conn.execute(
            "UPDATE albums SET cover_image_id = ?1 WHERE id = ?2",
            params![image_id, album_id],
        )?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("相册 {album_id} 不存在")));
        }
        Ok(())
    })
    .await
}

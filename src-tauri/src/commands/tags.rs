//! 标签相关命令。

use rusqlite::{Connection, OptionalExtension, params};
use tauri::State;

use crate::db::models::{Tag, validate_name};
use crate::db::{is_unique_violation, now_unix};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, with_db};

const TAG_SELECT: &str = "
    SELECT t.id,
           t.name,
           t.color,
           t.created_at,
           (SELECT COUNT(*) FROM image_tags it WHERE it.tag_id = t.id) AS image_count
    FROM tags t";

fn fetch_tag(conn: &Connection, id: i64) -> AppResult<Tag> {
    conn.query_row(&format!("{TAG_SELECT} WHERE t.id = ?1"), [id], Tag::from_row)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("标签 {id} 不存在")))
}

fn map_unique(err: rusqlite::Error, name: &str) -> AppError {
    if is_unique_violation(&err) {
        AppError::InvalidInput(format!("标签「{name}」已存在"))
    } else {
        AppError::Database(err)
    }
}

#[tauri::command]
pub async fn list_tags(state: State<'_, AppState>) -> AppResult<Vec<Tag>> {
    with_db(&state, |conn| {
        let mut stmt = conn.prepare(&format!("{TAG_SELECT} ORDER BY t.name COLLATE NOCASE"))?;
        let rows = stmt.query_map([], Tag::from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await
}

#[tauri::command]
pub async fn create_tag(
    state: State<'_, AppState>,
    name: String,
    color: Option<String>,
) -> AppResult<Tag> {
    let name = validate_name("标签名", &name)?;

    with_db(&state, move |conn| {
        conn.execute(
            "INSERT INTO tags (name, color, created_at) VALUES (?1, ?2, ?3)",
            params![name, color, now_unix()],
        )
        .map_err(|err| map_unique(err, &name))?;
        fetch_tag(conn, conn.last_insert_rowid())
    })
    .await
}

#[tauri::command]
pub async fn update_tag(
    state: State<'_, AppState>,
    id: i64,
    name: String,
    color: Option<String>,
) -> AppResult<Tag> {
    let name = validate_name("标签名", &name)?;

    with_db(&state, move |conn| {
        let affected = conn
            .execute(
                "UPDATE tags SET name = ?1, color = ?2 WHERE id = ?3",
                params![name, color, id],
            )
            .map_err(|err| map_unique(err, &name))?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("标签 {id} 不存在")));
        }
        fetch_tag(conn, id)
    })
    .await
}

#[tauri::command]
pub async fn delete_tag(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    with_db(&state, move |conn| {
        let affected = conn.execute("DELETE FROM tags WHERE id = ?1", [id])?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("标签 {id} 不存在")));
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn add_tag_to_image(
    state: State<'_, AppState>,
    image_id: i64,
    tag_id: i64,
) -> AppResult<()> {
    with_db(&state, move |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO image_tags (image_id, tag_id) VALUES (?1, ?2)",
            params![image_id, tag_id],
        )?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn remove_tag_from_image(
    state: State<'_, AppState>,
    image_id: i64,
    tag_id: i64,
) -> AppResult<()> {
    with_db(&state, move |conn| {
        conn.execute(
            "DELETE FROM image_tags WHERE image_id = ?1 AND tag_id = ?2",
            params![image_id, tag_id],
        )?;
        Ok(())
    })
    .await
}

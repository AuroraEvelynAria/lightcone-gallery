//! 图片相关命令：分页查询、详情、收藏 / 评分、缩略图。

use std::path::PathBuf;

use rusqlite::types::Value;
use rusqlite::{OptionalExtension, params_from_iter};
use tauri::State;

use crate::db::models::{
    AlbumBrief, Image, ImageDetail, ImageQuery, Orientation, PagedImages, Tag,
};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, with_db};
use crate::thumbnails;

/// 所有查询命令共用的列清单，保证 `Image::from_row` 能按名字取到每一列。
const IMAGE_COLUMNS: &str = "
    id, library_id, path, filename, extension, width, height,
    file_size, modified_at, taken_at, is_favorite, rating, indexed_at, pid";

/// LIKE 通配符转义，配合 `ESCAPE '\'` 使用，避免用户输入的 % 被当成通配符。
fn escape_like(term: &str) -> String {
    term.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[tauri::command]
pub async fn list_images(
    state: State<'_, AppState>,
    query: ImageQuery,
) -> AppResult<PagedImages> {
    let query = query.normalized();

    with_db(&state, move |conn| {
        let mut filter = String::from(" WHERE 1 = 1");
        let mut binds: Vec<Value> = Vec::new();

        if let Some(library_id) = query.library_id {
            filter.push_str(" AND library_id = ?");
            binds.push(Value::Integer(library_id));
        }
        if let Some(album_id) = query.album_id {
            filter.push_str(" AND id IN (SELECT image_id FROM album_images WHERE album_id = ?)");
            binds.push(Value::Integer(album_id));
        }
        if let Some(tag_id) = query.tag_id {
            filter.push_str(" AND id IN (SELECT image_id FROM image_tags WHERE tag_id = ?)");
            binds.push(Value::Integer(tag_id));
        }
        // 画师：一个作品只有一个画师，多个即「任一」（OR）
        if !query.artist_uids.is_empty() {
            let marks = std::iter::repeat("?")
                .take(query.artist_uids.len())
                .collect::<Vec<_>>()
                .join(", ");
            filter.push_str(&format!(" AND artist_uid IN ({marks})"));
            binds.extend(
                query
                    .artist_uids
                    .iter()
                    .map(|uid| Value::Text(uid.clone())),
            );
        }
        // 标签：多个是「同时包含」（AND），多选标签是为了收窄而不是发散
        if !query.pixiv_tags.is_empty() {
            let marks = std::iter::repeat("?")
                .take(query.pixiv_tags.len())
                .collect::<Vec<_>>()
                .join(", ");
            filter.push_str(&format!(
                " AND pid IN (
                     SELECT pid FROM illust_tags WHERE tag IN ({marks})
                     GROUP BY pid HAVING COUNT(DISTINCT tag) = {}
                 )",
                query.pixiv_tags.len()
            ));
            binds.extend(
                query
                    .pixiv_tags
                    .iter()
                    .map(|tag| Value::Text(tag.clone())),
            );
        }
        // 画幅：宽高在扫描时就存下来了，直接比列即可
        if let Some(orientation) = query.orientation {
            filter.push_str(match orientation {
                Orientation::Landscape => " AND width > height",
                Orientation::Portrait => " AND width < height",
                Orientation::Square => " AND width = height",
            });
        }
        if query.favorites_only {
            filter.push_str(" AND is_favorite = 1");
        }
        if let Some(minimum) = query.minimum_rating {
            filter.push_str(" AND rating >= ?");
            binds.push(Value::Integer(minimum));
        }
        if let Some(term) = query.search.as_deref() {
            filter.push_str(" AND (filename LIKE ? ESCAPE '\\' OR path LIKE ? ESCAPE '\\')");
            let pattern = format!("%{}%", escape_like(term));
            binds.push(Value::Text(pattern.clone()));
            binds.push(Value::Text(pattern));
        }

        let total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM images{filter}"),
            params_from_iter(binds.iter()),
            |row| row.get(0),
        )?;

        let direction = if query.sort_desc { "DESC" } else { "ASC" };
        let sql = format!(
            "SELECT {IMAGE_COLUMNS} FROM images{filter} \
             ORDER BY {} {direction}, id DESC LIMIT ? OFFSET ?",
            query.sort_by.column()
        );

        binds.push(Value::Integer(query.limit));
        binds.push(Value::Integer(query.offset));

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(binds.iter()), Image::from_row)?;
        let items = rows.collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(PagedImages {
            items,
            total,
            limit: query.limit,
            offset: query.offset,
        })
    })
    .await
}

#[tauri::command]
pub async fn get_image(state: State<'_, AppState>, id: i64) -> AppResult<ImageDetail> {
    with_db(&state, move |conn| {
        let image = conn
            .query_row(
                &format!("SELECT {IMAGE_COLUMNS} FROM images WHERE id = ?1"),
                [id],
                Image::from_row,
            )
            .optional()?
            .ok_or_else(|| AppError::NotFound(format!("图片 {id} 不存在")))?;

        let mut tag_stmt = conn.prepare(
            "SELECT t.id, t.name, t.color, t.created_at,
                    (SELECT COUNT(*) FROM image_tags it WHERE it.tag_id = t.id) AS image_count
             FROM tags t
             JOIN image_tags it ON it.tag_id = t.id
             WHERE it.image_id = ?1
             ORDER BY t.name COLLATE NOCASE",
        )?;
        let tags = tag_stmt
            .query_map([id], Tag::from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut album_stmt = conn.prepare(
            "SELECT a.id, a.name
             FROM albums a
             JOIN album_images ai ON ai.album_id = a.id
             WHERE ai.image_id = ?1
             ORDER BY a.name COLLATE NOCASE",
        )?;
        let albums = album_stmt
            .query_map([id], |row| {
                Ok(AlbumBrief {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(ImageDetail { image, tags, albums })
    })
    .await
}

#[tauri::command]
pub async fn toggle_favorite(state: State<'_, AppState>, id: i64) -> AppResult<bool> {
    with_db(&state, move |conn| {
        let affected = conn.execute(
            "UPDATE images SET is_favorite = CASE is_favorite WHEN 1 THEN 0 ELSE 1 END
             WHERE id = ?1",
            [id],
        )?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("图片 {id} 不存在")));
        }
        let value: bool = conn.query_row("SELECT is_favorite FROM images WHERE id = ?1", [id], |row| {
            row.get(0)
        })?;
        Ok(value)
    })
    .await
}

#[tauri::command]
pub async fn set_rating(state: State<'_, AppState>, id: i64, rating: i64) -> AppResult<i64> {
    if !(0..=5).contains(&rating) {
        return Err(AppError::InvalidInput("评分必须在 0 到 5 之间".into()));
    }

    with_db(&state, move |conn| {
        let affected = conn.execute("UPDATE images SET rating = ?1 WHERE id = ?2", [rating, id])?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("图片 {id} 不存在")));
        }
        Ok(rating)
    })
    .await
}

/// 返回缩略图的绝对路径（有损 WebP）。前端拿到后用 `convertFileSrc` 转成 asset:// URL。
#[tauri::command]
pub async fn thumbnail_for(
    state: State<'_, AppState>,
    id: i64,
    size: Option<u32>,
) -> AppResult<String> {
    let source: PathBuf = with_db(&state, move |conn| {
        conn.query_row("SELECT path FROM images WHERE id = ?1", [id], |row| {
            row.get::<_, String>(0)
        })
        .optional()?
        .map(PathBuf::from)
        .ok_or_else(|| AppError::NotFound(format!("图片 {id} 不存在")))
    })
    .await?;

    let size = thumbnails::normalize_size(size.unwrap_or(thumbnails::DEFAULT_SIZE));
    thumbnails::ensure_offloaded(
        state.thumbnail_dir.clone(),
        thumbnails::Owner::Image,
        id,
        source,
        size,
    )
    .await
}

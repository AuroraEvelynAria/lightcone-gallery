//! 与前端共享的数据模型。序列化统一使用 camelCase，前端可直接得到惯用的 TS 类型。

use rusqlite::Row;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

// ---------------------------------------------------------------------------
// 图库
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub image_count: i64,
    pub created_at: i64,
    pub last_scanned_at: Option<i64>,
}

impl Library {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            path: row.get("path")?,
            name: row.get("name")?,
            image_count: row.get("image_count")?,
            created_at: row.get("created_at")?,
            last_scanned_at: row.get("last_scanned_at")?,
        })
    }
}

// ---------------------------------------------------------------------------
// 图片
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    pub id: i64,
    pub library_id: i64,
    pub path: String,
    pub filename: String,
    pub extension: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub file_size: i64,
    pub modified_at: i64,
    pub taken_at: Option<i64>,
    pub is_favorite: bool,
    pub rating: i64,
    pub indexed_at: i64,
    /// 从文件名反推的 Pixiv 作品号（`{pid}_p{n}`），不符合命名时为 None。
    /// 它是通用图库通往 Pixiv 维度（`illust_meta` / `illust_tags`）的唯一桥梁。
    pub pid: Option<String>,
}

impl Image {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            library_id: row.get("library_id")?,
            path: row.get("path")?,
            filename: row.get("filename")?,
            extension: row.get("extension")?,
            width: row.get("width")?,
            height: row.get("height")?,
            file_size: row.get("file_size")?,
            modified_at: row.get("modified_at")?,
            taken_at: row.get("taken_at")?,
            is_favorite: row.get("is_favorite")?,
            rating: row.get("rating")?,
            indexed_at: row.get("indexed_at")?,
            pid: row.get("pid")?,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageDetail {
    #[serde(flatten)]
    pub image: Image,
    pub tags: Vec<Tag>,
    pub albums: Vec<AlbumBrief>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumBrief {
    pub id: i64,
    pub name: String,
}

// ---------------------------------------------------------------------------
// 查询
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortBy {
    Filename,
    #[default]
    ModifiedAt,
    IndexedAt,
    FileSize,
    Rating,
}

impl SortBy {
    /// 列名白名单。排序字段来自前端，必须以映射方式落地，不能拼接原始字符串。
    pub fn column(self) -> &'static str {
        match self {
            SortBy::Filename => "filename COLLATE NOCASE",
            SortBy::ModifiedAt => "modified_at",
            SortBy::IndexedAt => "indexed_at",
            SortBy::FileSize => "file_size",
            SortBy::Rating => "rating",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageQuery {
    pub library_id: Option<i64>,
    pub album_id: Option<i64>,
    pub tag_id: Option<i64>,
    /// 按画师筛选（多选，任一匹配）。画师从路径反推，不需要联网。
    pub artist_uids: Vec<String>,
    /// 按 Pixiv 官方标签筛选（多选，**同时包含**才算命中）。
    pub pixiv_tags: Vec<String>,
    /// 按文件名 / 路径模糊匹配
    pub search: Option<String>,
    pub favorites_only: bool,
    pub minimum_rating: Option<i64>,
    pub sort_by: SortBy,
    pub sort_desc: bool,
    pub limit: i64,
    pub offset: i64,
}

impl Default for ImageQuery {
    fn default() -> Self {
        Self {
            library_id: None,
            album_id: None,
            tag_id: None,
            artist_uids: Vec::new(),
            pixiv_tags: Vec::new(),
            search: None,
            favorites_only: false,
            minimum_rating: None,
            sort_by: SortBy::default(),
            sort_desc: true,
            limit: 200,
            offset: 0,
        }
    }
}

impl ImageQuery {
    /// 归一化来自前端的取值，避免超大 limit / 负 offset 打到 SQLite。
    pub fn normalized(mut self) -> Self {
        self.limit = self.limit.clamp(1, 1000);
        self.offset = self.offset.max(0);
        self.search = self
            .search
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        self.artist_uids = normalize_list(self.artist_uids);
        self.pixiv_tags = normalize_list(self.pixiv_tags);
        self
    }
}

/// 清空空白项并去重，避免前端传来空串把 SQL 拼出个空 IN 列表。
fn normalize_list(values: Vec<String>) -> Vec<String> {
    let mut cleaned: Vec<String> = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    cleaned.sort();
    cleaned.dedup();
    cleaned
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PagedImages {
    pub items: Vec<Image>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

// ---------------------------------------------------------------------------
// 相册
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub cover_image_id: Option<i64>,
    pub image_count: i64,
    pub created_at: i64,
}

impl Album {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            description: row.get("description")?,
            cover_image_id: row.get("cover_image_id")?,
            image_count: row.get("image_count")?,
            created_at: row.get("created_at")?,
        })
    }
}

// ---------------------------------------------------------------------------
// 标签
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub image_count: i64,
    pub created_at: i64,
}

impl Tag {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            color: row.get("color")?,
            image_count: row.get("image_count")?,
            created_at: row.get("created_at")?,
        })
    }
}

// ---------------------------------------------------------------------------
// 概览
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub images: i64,
    pub libraries: i64,
    pub albums: i64,
    pub tags: i64,
    pub favorites: i64,
    pub total_bytes: i64,
}

// ---------------------------------------------------------------------------
// 索引进度
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScanPhase {
    Discovering,
    Indexing,
    Finished,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub library_id: i64,
    pub library_name: String,
    pub phase: ScanPhase,
    pub processed: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub library_id: i64,
    pub discovered: usize,
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub removed: usize,
    /// 扩展名可识别、但当前解码器读不出尺寸的文件（如 HEIC / RAW）
    pub skipped: usize,
}

// ---------------------------------------------------------------------------
// 作品（阶段一：Pixiv 维度的索引）
// ---------------------------------------------------------------------------

/// 一行代表「某作品的某一页」。文件名形如 `{pid}_p{num}.jpg`。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Artwork {
    pub id: i64,
    pub pid: String,
    /// 原图绝对路径。前端只在点击卡片弹窗时才用它。
    pub original_path: String,
    /// 缩略图绝对路径。扫描阶段已生成，列表里直接带着走，前端不必为每张卡片
    /// 再问一次后端。生成失败或缓存被清掉时为 `None`，前端会走一次重生成回退。
    pub thumbnail_path: Option<String>,
    /// 文件名里的 `_p{num}`
    pub page: i64,
    pub created_at: i64,
}

impl Artwork {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            pid: row.get("pid")?,
            original_path: row.get("original_path")?,
            thumbnail_path: row.get("thumbnail_path")?,
            page: row.get("page")?,
            created_at: row.get("created_at")?,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtworkPage {
    pub items: Vec<Artwork>,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtworkScanSummary {
    /// 目录里发现的图片文件总数
    pub discovered: usize,
    /// 文件名符合 `{pid}_p{num}` 的数量
    pub matched: usize,
    /// 实际写入的行数
    pub inserted: usize,
    /// 因 (pid, page) 已存在而被去重掉的数量
    pub duplicates: usize,
    /// 命名不符合规范、未收进作品库的数量
    pub skipped: usize,
    /// 在忽略名单里、被主动跳过的数量（之前被移出过作品库）
    pub ignored: usize,
    /// 本次真正生成 / 重算了缩略图的张数
    pub thumbnails_generated: usize,
    /// 缩略图缓存仍然新鲜、直接跳过的张数
    pub thumbnails_skipped: usize,
    /// 缩略图生成失败的张数（源文件缺失或解码失败）
    pub thumbnails_failed: usize,
}

/// 一条作品的 Pixiv 侧信息，供两个检视弹窗显示。
///
/// 与 `ImageDetail` 分开：那是「主人自己的数据」（标签 / 相册 / 收藏），
/// 这是「Pixiv 上的数据」（标题 / 画师 / 官方标签），来源与更新时机都不同。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IllustDetailView {
    pub pid: String,
    pub title: String,
    pub artist_uid: String,
    pub artist_name: String,
    pub artist_account: String,
    pub page_count: i64,
    /// 0 全年龄 / 1 R-18 / 2 R-18G
    pub x_restrict: i64,
    /// 0 插画 / 1 漫画 / 2 动图
    pub illust_type: i64,
    pub create_date: String,
    pub tags: Vec<String>,
}

/// 画师及其作品数。
///
/// `name` 只是**显示标签**：同一个 uid 在本地数据里出现过多个历史名字（画师改过名、
/// 或下载时把展会后缀写进了目录名），`artists` 表按先到先得保留最早那个。
/// 界面要做分组时请按 `uid` 分组，不要假设名字唯一。见 `crate::artists`。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistSummary {
    /// Pixiv 用户 ID，稳定身份
    pub uid: String,
    /// 显示用的名字，可能不是最新的那个
    pub name: String,
    /// 通用图库（`images`）里的张数
    pub images: usize,
    /// 作品库（`artworks`）里的条数
    pub artworks: usize,
}

/// 扫描阶段二的缩略图批量生成结果。只用于内部统计，不直接发给前端。
#[derive(Debug, Clone, Copy, Default)]
pub struct ThumbnailBatch {
    /// 本次真正写盘的张数
    pub generated: usize,
    /// 缓存仍然新鲜、直接跳过的张数
    pub skipped: usize,
    /// 生成失败的张数
    pub failed: usize,
}

// ---------------------------------------------------------------------------
// 输入校验
// ---------------------------------------------------------------------------

/// 标签 / 相册名等短文本的统一校验。
pub fn validate_name(field: &str, name: &str) -> AppResult<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::InvalidInput(format!("{field}不能为空")));
    }
    if trimmed.chars().count() > 120 {
        return Err(AppError::InvalidInput(format!("{field}过长（最多 120 字符）")));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_normalization_clamps_bounds() {
        let raw = ImageQuery {
            limit: 999_999,
            offset: -5,
            search: Some("   ".into()),
            ..Default::default()
        }
        .normalized();

        assert_eq!(raw.limit, 1000);
        assert_eq!(raw.offset, 0);
        assert_eq!(raw.search, None, "空白搜索词应被丢弃");
    }

    #[test]
    fn sort_columns_are_whitelisted() {
        assert_eq!(SortBy::Filename.column(), "filename COLLATE NOCASE");
        assert_eq!(SortBy::FileSize.column(), "file_size");
    }

    #[test]
    fn validate_name_rejects_blank_and_accepts_trimmed() {
        assert!(validate_name("标签名", "   ").is_err());
        assert_eq!(validate_name("标签名", "  风景  ").unwrap(), "风景");
    }
}

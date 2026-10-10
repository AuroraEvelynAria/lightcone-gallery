/**
 * 与 Rust 端 `src-tauri/src/db/models.rs` 一一对应的类型定义。
 * 后端统一用 camelCase 序列化，改动模型时请同步两处。
 */

export interface Library {
  id: number;
  path: string;
  name: string;
  imageCount: number;
  createdAt: number;
  lastScannedAt: number | null;
}

export interface Image {
  id: number;
  libraryId: number;
  path: string;
  filename: string;
  extension: string;
  width: number | null;
  height: number | null;
  fileSize: number;
  modifiedAt: number;
  takenAt: number | null;
  isFavorite: boolean;
  rating: number;
  indexedAt: number;
}

export interface ImageDetail extends Image {
  tags: Tag[];
  albums: AlbumBrief[];
}

export interface AlbumBrief {
  id: number;
  name: string;
}

export interface Album {
  id: number;
  name: string;
  description: string | null;
  coverImageId: number | null;
  imageCount: number;
  createdAt: number;
}

export interface Tag {
  id: number;
  name: string;
  color: string | null;
  imageCount: number;
  createdAt: number;
}

export interface Stats {
  images: number;
  libraries: number;
  albums: number;
  tags: number;
  favorites: number;
  totalBytes: number;
}

export type SortBy = "filename" | "modifiedAt" | "indexedAt" | "fileSize" | "rating";

export interface ImageQuery {
  libraryId?: number | null;
  albumId?: number | null;
  tagId?: number | null;
  search?: string | null;
  favoritesOnly?: boolean;
  minimumRating?: number | null;
  sortBy?: SortBy;
  sortDesc?: boolean;
  limit?: number;
  offset?: number;
}

export interface PagedImages {
  items: Image[];
  total: number;
  limit: number;
  offset: number;
}

export interface ScanSummary {
  libraryId: number;
  discovered: number;
  added: number;
  updated: number;
  unchanged: number;
  removed: number;
  skipped: number;
}

export type ScanPhase = "discovering" | "indexing" | "finished";

export interface ScanProgress {
  libraryId: number;
  libraryName: string;
  phase: ScanPhase;
  processed: number;
  total: number;
}

export interface CacheUsage {
  bytes: number;
  files: number;
}

// ---------------------------------------------------------------------------
// 阶段一：作品（Pixiv）维度
// ---------------------------------------------------------------------------

/** 一行代表「某作品的某一页」，文件名形如 `{pid}_p{num}.jpg`。 */
export interface Artwork {
  id: number;
  pid: string;
  /** 原图绝对路径。只在点击卡片弹窗时使用，列表本身不加载它。 */
  originalPath: string;
  /**
   * 缩略图绝对路径。扫描阶段就生成好了，列表直接带着走，卡片不必再问后端。
   * 为 `null` 表示生成失败或缓存被清掉，卡片会走一次重生成回退。
   */
  thumbnailPath: string | null;
  /** 文件名里的 `_p{num}` */
  page: number;
  createdAt: number;
}

export interface ArtworkPage {
  items: Artwork[];
  total: number;
  offset: number;
  limit: number;
}

export interface ArtworkScanSummary {
  /** 目录里发现的图片文件总数 */
  discovered: number;
  /** 文件名符合 `{pid}_p{num}` 的数量 */
  matched: number;
  /** 实际写入的行数 */
  inserted: number;
  /** 因 (pid, page) 已存在而被去重掉的数量 */
  duplicates: number;
  /** 命名不符合规范、未收进作品库的数量 */
  skipped: number;
  /** 在忽略名单里、被主动跳过的数量（之前被移出过作品库） */
  ignored: number;
  /** 本次真正生成 / 重算了缩略图的张数 */
  thumbnailsGenerated: number;
  /** 缩略图缓存仍然新鲜、直接跳过的张数 */
  thumbnailsSkipped: number;
  /** 缩略图生成失败的张数（源文件缺失或解码失败） */
  thumbnailsFailed: number;
}

/** 缩略图批量生成的进度（`artworks://thumbnail-progress` 事件的负载）。 */
export interface ThumbnailProgress {
  processed: number;
  total: number;
}

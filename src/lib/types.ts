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
  /**
   * 从文件名反推的 Pixiv 作品号（`{pid}_p{n}`），不符合命名时为 null。
   * 通用图库靠它去查 `illust_tags` / `illust_meta`。
   */
  pid: string | null;
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
  /** 按画师筛选（多选，任一匹配）。画师从路径反推，不需要联网。 */
  artistUids?: string[] | null;
  /** 按 Pixiv 官方标签筛选（多选，**同时包含**才算命中）。 */
  pixivTags?: string[] | null;
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

// ---------------------------------------------------------------------------
// 画师与联网元数据
// ---------------------------------------------------------------------------

/**
 * 画师及其作品数。
 *
 * `name` 只是**显示标签**：同一个 uid 在本地数据里出现过多个历史名字（画师改过名、
 * 或下载时把展会后缀写进了目录名），后端按先到先得保留最早那个；联网补全元数据后
 * 会被接口给的**未清洗原名**覆盖（路径里的目录名是 Windows 文件名，`/` 这类字符
 * 被删掉了）。分组请按 `uid`，不要假设名字唯一。
 */
export interface ArtistSummary {
  uid: string;
  name: string;
  /** 通用图库（images）里的张数 */
  images: number;
  /** 作品库（artworks）里的条数 */
  artworks: number;
}

/** 一个 Pixiv 标签 + 用它标过的作品数。标签只能联网取到，路径里没有。 */
export interface TagCount {
  tag: string;
  count: number;
}

/** 元数据覆盖情况。 */
export interface MetaStats {
  /** 成功取到的 PID 数 */
  done: number;
  /** 抓过但失败的（含作品已删除） */
  failed: number;
  /** 还没抓过的 */
  pending: number;
}

/** 一批元数据抓取的结果。 */
export interface MetaBatch {
  fetched: number;
  missing: number;
  failed: number;
  /** 连续失败过多而提前中止（通常是代理没开） */
  aborted: boolean;
  /** 中止的原因是**被限流**，而不是网络不通。界面上要提示「过一阵再来」。 */
  rateLimited: boolean;
}

/** 元数据抓取进度（`pixiv://meta-progress` 事件的负载）。 */
export interface MetaProgress {
  processed: number;
  total: number;
}

/**
 * 一条作品的 Pixiv 侧信息。
 *
 * 与 `ImageDetail` 分开：那是「主人自己的数据」（自己的标签 / 相册 / 收藏），
 * 这是「Pixiv 上的数据」（标题 / 画师 / 官方标签），来源与更新时机都不同。
 */
export interface IllustDetailView {
  pid: string;
  title: string;
  artistUid: string;
  /** 接口给的未清洗原名（路径里的目录名会被 Windows 吃掉 `/` 这类字符） */
  artistName: string;
  artistAccount: string;
  pageCount: number;
  /** 0 全年龄 / 1 R-18 / 2 R-18G */
  xRestrict: number;
  /** 0 插画 / 1 漫画 / 2 动图 */
  illustType: number;
  createDate: string;
  tags: string[];
}

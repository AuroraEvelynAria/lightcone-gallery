/**
 * Tauri 命令的类型化封装。所有 IPC 调用都收口在这里，组件不直接 invoke。
 *
 * 约定：Rust 命令名用 snake_case，参数用 camelCase（Tauri 会自动转换命名风格）。
 */

import { invoke } from "@tauri-apps/api/core";

import type {
  Album,
  Artwork,
  ArtistSummary,
  ArtworkPage,
  ArtworkScanSummary,
  CacheUsage,
  IllustDetailView,
  Image,
  ImageDetail,
  ImageQuery,
  Library,
  PagedImages,
  MetaBatch,
  MetaProgress,
  MetaStats,
  ScanProgress,
  ScanSummary,
  SortBy,
  Stats,
  Tag,
  TagCount,
  ThumbnailProgress,
} from "./types";

/** 扫描进度事件的频道名，与 Rust 端 `SCAN_PROGRESS_EVENT` 保持一致。 */
export const SCAN_PROGRESS_EVENT = "scan://progress";

/** 作品缩略图生成进度事件的频道名，与 Rust 端 `THUMBNAIL_PROGRESS_EVENT` 保持一致。 */
export const THUMBNAIL_PROGRESS_EVENT = "artworks://thumbnail-progress";

/** 元数据抓取进度事件的频道名，与 Rust 端 `META_PROGRESS_EVENT` 保持一致。 */
export const META_PROGRESS_EVENT = "pixiv://meta-progress";

export type { MetaProgress, ScanProgress, ThumbnailProgress };

export const api = {
  // ---- 图库 ---------------------------------------------------------------
  listLibraries: () => invoke<Library[]>("list_libraries"),
  addLibrary: (path: string) => invoke<Library>("add_library", { path }),
  removeLibrary: (id: number) => invoke<void>("remove_library", { id }),
  scanLibrary: (id: number) => invoke<ScanSummary>("scan_library", { id }),
  scanAllLibraries: () => invoke<ScanSummary[]>("scan_all_libraries"),

  // ---- 图片 ---------------------------------------------------------------
  listImages: (query: ImageQuery = {}) => invoke<PagedImages>("list_images", { query }),
  getImage: (id: number) => invoke<ImageDetail>("get_image", { id }),
  toggleFavorite: (id: number) => invoke<boolean>("toggle_favorite", { id }),
  setRating: (id: number, rating: number) => invoke<number>("set_rating", { id, rating }),
  thumbnailFor: (id: number, size = 320) => invoke<string>("thumbnail_for", { id, size }),

  // ---- 相册 ---------------------------------------------------------------
  listAlbums: () => invoke<Album[]>("list_albums"),
  createAlbum: (name: string, description?: string) =>
    invoke<Album>("create_album", { name, description: description ?? null }),
  renameAlbum: (id: number, name: string) => invoke<Album>("rename_album", { id, name }),
  deleteAlbum: (id: number) => invoke<void>("delete_album", { id }),
  addImagesToAlbum: (albumId: number, imageIds: number[]) =>
    invoke<number>("add_images_to_album", { albumId, imageIds }),
  removeImagesFromAlbum: (albumId: number, imageIds: number[]) =>
    invoke<number>("remove_images_from_album", { albumId, imageIds }),
  setAlbumCover: (albumId: number, imageId: number | null) =>
    invoke<void>("set_album_cover", { albumId, imageId }),

  // ---- 标签 ---------------------------------------------------------------
  listTags: () => invoke<Tag[]>("list_tags"),
  createTag: (name: string, color?: string | null) =>
    invoke<Tag>("create_tag", { name, color: color ?? null }),
  updateTag: (id: number, name: string, color?: string | null) =>
    invoke<Tag>("update_tag", { id, name, color: color ?? null }),
  deleteTag: (id: number) => invoke<void>("delete_tag", { id }),
  addTagToImage: (imageId: number, tagId: number) =>
    invoke<void>("add_tag_to_image", { imageId, tagId }),
  removeTagFromImage: (imageId: number, tagId: number) =>
    invoke<void>("remove_tag_from_image", { imageId, tagId }),

  // ---- 系统 ---------------------------------------------------------------
  stats: () => invoke<Stats>("library_stats"),
  thumbnailCacheUsage: () => invoke<CacheUsage>("thumbnail_cache_usage"),
  clearThumbnailCache: () => invoke<void>("clear_thumbnail_cache"),

  // ---- 外观（阶段三）------------------------------------------------------
  /**
   * 应用窗口材质。`material` 取 `solid` / `mica` / `acrylic` / `tabbed` / `blur`，
   * `dark` 用来选材质的明暗变体（只有 Mica / Tabbed 提供），
   * `tint` 是染色（`#rrggbb`，只有 Acrylic / Blur 吃）。
   *
   * 注意：底层 `window_vibrancy` 的错误被 Tauri 丢掉了，所以不支持的平台上
   * 这个调用会「成功」但什么也没发生 —— 平台要求由前端标注，不假装能检测。
   */
  setWindowMaterial: (material: string, dark: boolean, tint?: string) =>
    invoke<void>("set_window_material", { material, dark, tint: tint ?? null }),
  /** 读取全部键值设置。数据库是外观偏好的权威来源，localStorage 只作首帧缓存。 */
  loadSettings: () => invoke<Record<string, string>>("load_settings"),
  /** 写入若干键值设置（同名的键直接覆盖）。 */
  saveSettings: (entries: Record<string, string>) =>
    invoke<void>("save_settings", { entries }),

  // ---- 作品（阶段一：Pixiv 维度）------------------------------------------
  /** 扫描一个本地文件夹，解析 `{pid}_p{num}` 入库，并为缺缩略图的作品生成 WebP */
  scanArtworksFolder: (path: string) =>
    invoke<ArtworkScanSummary>("scan_artworks_folder", { path }),
  /**
   * 分页读取作品列表：从第 offset 条开始返回 limit 条。
   *
   * 每条都带着 `originalPath` 与 `thumbnailPath`，卡片直接用后者渲染，无需额外 IPC。
   */
  listArtworks: (
    offset = 0,
    limit = 60,
    filters?: { artistUids?: string[]; pixivTags?: string[] },
  ) =>
    invoke<ArtworkPage>("list_artworks", {
      offset,
      limit,
      artistUids: filters?.artistUids ?? [],
      pixivTags: filters?.pixivTags ?? [],
    }),
  /**
   * 重新生成作品缩略图并把新路径写回数据库。
   *
   * 正常路径上不该调它 —— 列表里已经带着 `thumbnailPath`。这是自愈回退：
   * 缓存被清空 / 文件被删导致 `<img>` 加载失败时调用一次。
   */
  thumbnailForArtwork: (id: number) => invoke<string>("thumbnail_for_artwork", { id }),
  /**
   * 把作品移出作品库：删记录与缩略图缓存，**不删除原图**。
   *
   * 顺带把 `(pid, page)` 记进忽略名单，重扫同一目录不会再收回来。
   * 返回实际删掉的行数（传进来的 id 可能有一部分已经不存在）。
   */
  removeArtworks: (ids: number[]) => invoke<number>("remove_artworks", { ids }),
  /** 列出所有画师及各自的作品数，按总量倒序。画师从路径反推，不联网。 */
  listArtists: () => invoke<ArtistSummary[]>("list_artists"),
  /**
   * 库里还有多少 PID 没成功取过元数据。
   * 「成功」认的是 `error IS NULL` —— 抓过但失败的不算。
   */
  countMissingMeta: () => invoke<number>("count_missing_meta"),
  /**
   * 取一批还没成功抓过的 PID，联网补全元数据（标题 / 画师 / 标签）并落库。
   *
   * 后端串行 + 每次间隔 800ms，非 200 不重试，连续失败 5 次中止整批；
   * `limit` 上限 300。要补全整个库请分批点，不要一次跑几十分钟。
   * 进度通过 `META_PROGRESS_EVENT` 推送。
   */
  fetchMissingMeta: (limit?: number) =>
    invoke<MetaBatch>("fetch_missing_meta", { limit: limit ?? null }),
  /** 按标签统计，倒序。标签只能联网取到。 */
  listPixivTags: (limit?: number) =>
    invoke<TagCount[]>("list_pixiv_tags", { limit: limit ?? null }),
  /** 元数据覆盖情况（已补全 / 失败 / 待补全）。 */
  pixivMetaStats: () => invoke<MetaStats>("pixiv_meta_stats"),
  /**
   * 取一条作品的 Pixiv 侧信息（标题 / 画师 / 官方标签），供检视弹窗显示。
   * 返回 null 表示这条还没补全过元数据 —— 界面要显示「还没取过」而不是「没有标签」。
   */
  illustDetail: (pid: string) =>
    invoke<IllustDetailView | null>("illust_detail", { pid }),
  /** 忽略名单里有多少条（之前被移出过作品库的 `(pid, page)`）。 */
  countIgnoredArtworks: () => invoke<number>("count_ignored_artworks"),
  /** 清空忽略名单，返回清掉的条数。之后重扫目录会把它们再收回来。 */
  clearIgnoredArtworks: () => invoke<number>("clear_ignored_artworks"),
};

export type {
  Album,
  Artwork,
  ArtworkPage,
  ArtworkScanSummary,
  Image,
  ImageDetail,
  ImageQuery,
  Library,
  PagedImages,
  SortBy,
  Stats,
  Tag,
};

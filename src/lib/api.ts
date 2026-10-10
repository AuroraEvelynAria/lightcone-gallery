/**
 * Tauri 命令的类型化封装。所有 IPC 调用都收口在这里，组件不直接 invoke。
 *
 * 约定：Rust 命令名用 snake_case，参数用 camelCase（Tauri 会自动转换命名风格）。
 */

import { invoke } from "@tauri-apps/api/core";

import type {
  Album,
  Artwork,
  ArtworkPage,
  ArtworkScanSummary,
  CacheUsage,
  Image,
  ImageDetail,
  ImageQuery,
  Library,
  PagedImages,
  ScanProgress,
  ScanSummary,
  SortBy,
  Stats,
  Tag,
  ThumbnailProgress,
} from "./types";

/** 扫描进度事件的频道名，与 Rust 端 `SCAN_PROGRESS_EVENT` 保持一致。 */
export const SCAN_PROGRESS_EVENT = "scan://progress";

/** 作品缩略图生成进度事件的频道名，与 Rust 端 `THUMBNAIL_PROGRESS_EVENT` 保持一致。 */
export const THUMBNAIL_PROGRESS_EVENT = "artworks://thumbnail-progress";

export type { ScanProgress, ThumbnailProgress };

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
  listArtworks: (offset = 0, limit = 60) =>
    invoke<ArtworkPage>("list_artworks", { offset, limit }),
  /**
   * 重新生成作品缩略图并把新路径写回数据库。
   *
   * 正常路径上不该调它 —— 列表里已经带着 `thumbnailPath`。这是自愈回退：
   * 缓存被清空 / 文件被删导致 `<img>` 加载失败时调用一次。
   */
  thumbnailForArtwork: (id: number) => invoke<string>("thumbnail_for_artwork", { id }),
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

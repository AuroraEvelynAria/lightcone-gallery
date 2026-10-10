/**
 * Tauri 命令的类型化封装。所有 IPC 调用都收口在这里，组件不直接 invoke。
 *
 * 约定：Rust 命令名用 snake_case，参数用 camelCase（Tauri 会自动转换命名风格）。
 */

import { invoke } from "@tauri-apps/api/core";

import type {
  Album,
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
} from "./types";

/** 扫描进度事件的频道名，与 Rust 端 `SCAN_PROGRESS_EVENT` 保持一致。 */
export const SCAN_PROGRESS_EVENT = "scan://progress";

export type { ScanProgress };

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
};

export type { Image, ImageDetail, ImageQuery, Library, PagedImages, SortBy, Stats, Tag, Album };

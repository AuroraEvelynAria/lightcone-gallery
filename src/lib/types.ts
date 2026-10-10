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

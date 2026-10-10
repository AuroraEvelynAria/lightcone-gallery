/**
 * 全局图库状态（Svelte 5 runes）。
 *
 * 组件只从这里读写数据，不直接调用 api —— 这样「筛选条件 -> 查询 -> 列表」
 * 的同步逻辑只有一处，避免各组件各写一份。
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { api, SCAN_PROGRESS_EVENT } from "./api";
import type {
  Album,
  CacheUsage,
  Image,
  Library,
  ScanProgress,
  SortBy,
  Stats,
  Tag,
} from "./types";

export type Selection =
  | { kind: "all" }
  | { kind: "favorites" }
  | { kind: "library"; id: number }
  | { kind: "album"; id: number }
  | { kind: "tag"; id: number };

const PAGE_SIZE = 120;

class GalleryStore {
  libraries = $state<Library[]>([]);
  albums = $state<Album[]>([]);
  tags = $state<Tag[]>([]);
  stats = $state<Stats | null>(null);
  cacheUsage = $state<CacheUsage | null>(null);

  images = $state<Image[]>([]);
  total = $state(0);
  loading = $state(false);
  loadingMore = $state(false);

  selection = $state<Selection>({ kind: "all" });
  search = $state("");
  sortBy = $state<SortBy>("modifiedAt");
  sortDesc = $state(true);
  minimumRating = $state(0);

  scanning = $state(false);
  scanProgress = $state<ScanProgress | null>(null);

  error = $state<string | null>(null);
  notice = $state<string | null>(null);

  #unlisten: UnlistenFn | null = null;

  get hasMore(): boolean {
    return this.images.length < this.total;
  }

  /** 当前筛选条件的中文描述，用于空状态与标题。 */
  get selectionLabel(): string {
    // 取成局部常量：TypeScript 的类型收窄不会穿透进箭头函数回调
    const selection = this.selection;
    switch (selection.kind) {
      case "favorites":
        return "收藏";
      case "library":
        return this.libraries.find((item) => item.id === selection.id)?.name ?? "图库";
      case "album":
        return this.albums.find((item) => item.id === selection.id)?.name ?? "相册";
      case "tag":
        return this.tags.find((item) => item.id === selection.id)?.name ?? "标签";
      default:
        return "全部图片";
    }
  }

  async init(): Promise<void> {
    this.#unlisten ??= await listen<ScanProgress>(SCAN_PROGRESS_EVENT, (event) => {
      this.scanProgress = event.payload;
    });
    await this.refreshSidebar();
    await this.refreshCacheUsage();
    await this.refreshImages();
  }

  async refreshCacheUsage(): Promise<void> {
    try {
      this.cacheUsage = await api.thumbnailCacheUsage();
    } catch (err) {
      this.error = String(err);
    }
  }

  async clearThumbnailCache(): Promise<void> {
    try {
      await api.clearThumbnailCache();
      await this.refreshCacheUsage();
      // 磁盘缓存没了，重新取图会触发重建
      await this.refreshImages();
      this.notice = "缩略图缓存已清空";
    } catch (err) {
      this.error = String(err);
    }
  }

  async dispose(): Promise<void> {
    this.#unlisten?.();
    this.#unlisten = null;
  }

  #query(offset: number) {
    const selection = this.selection;
    const base = {
      search: this.search.trim() || null,
      sortBy: this.sortBy,
      sortDesc: this.sortDesc,
      minimumRating: this.minimumRating > 0 ? this.minimumRating : null,
      limit: PAGE_SIZE,
      offset,
    };

    switch (selection.kind) {
      case "library":
        return { ...base, libraryId: selection.id };
      case "album":
        return { ...base, albumId: selection.id };
      case "tag":
        return { ...base, tagId: selection.id };
      case "favorites":
        return { ...base, favoritesOnly: true };
      default:
        return base;
    }
  }

  async refreshImages(): Promise<void> {
    this.loading = true;
    this.error = null;
    try {
      const page = await api.listImages(this.#query(0));
      this.images = page.items;
      this.total = page.total;
    } catch (err) {
      this.error = String(err);
    } finally {
      this.loading = false;
    }
  }

  async loadMore(): Promise<void> {
    if (this.loading || this.loadingMore || !this.hasMore) return;
    this.loadingMore = true;
    try {
      const page = await api.listImages(this.#query(this.images.length));
      this.images = [...this.images, ...page.items];
      this.total = page.total;
    } catch (err) {
      this.error = String(err);
    } finally {
      this.loadingMore = false;
    }
  }

  async refreshSidebar(): Promise<void> {
    try {
      const [libraries, albums, tags, stats] = await Promise.all([
        api.listLibraries(),
        api.listAlbums(),
        api.listTags(),
        api.stats(),
      ]);
      this.libraries = libraries;
      this.albums = albums;
      this.tags = tags;
      this.stats = stats;
    } catch (err) {
      this.error = String(err);
    }
  }

  /** 切换筛选维度会自动重新拉取列表。 */
  select(selection: Selection): void {
    this.selection = selection;
    void this.refreshImages();
  }

  async addLibrary(): Promise<void> {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({
      directory: true,
      multiple: false,
      title: "选择图库根目录",
    });
    if (typeof picked !== "string") return;

    try {
      const library = await api.addLibrary(picked);
      await this.scan(library.id);
    } catch (err) {
      this.error = String(err);
    }
  }

  async removeLibrary(id: number): Promise<void> {
    try {
      await api.removeLibrary(id);
      if (this.selection.kind === "library" && this.selection.id === id) {
        this.selection = { kind: "all" };
      }
      await this.refreshSidebar();
      await this.refreshImages();
    } catch (err) {
      this.error = String(err);
    }
  }

  /** 不传 id 表示扫描全部图库。 */
  async scan(libraryId?: number): Promise<void> {
    this.scanning = true;
    this.error = null;
    this.scanProgress = null;
    try {
      const summaries =
        libraryId === undefined
          ? await api.scanAllLibraries()
          : [await api.scanLibrary(libraryId)];

      const added = summaries.reduce((sum, s) => sum + s.added, 0);
      const updated = summaries.reduce((sum, s) => sum + s.updated, 0);
      const removed = summaries.reduce((sum, s) => sum + s.removed, 0);
      this.notice = `索引完成：新增 ${added}、更新 ${updated}、移除 ${removed}`;

      await this.refreshSidebar();
      await this.refreshImages();
    } catch (err) {
      this.error = String(err);
    } finally {
      this.scanning = false;
      this.scanProgress = null;
    }
  }

  async toggleFavorite(image: Image): Promise<void> {
    try {
      image.isFavorite = await api.toggleFavorite(image.id);
      this.stats = await api.stats();
    } catch (err) {
      this.error = String(err);
    }
  }

  async setRating(image: Image, rating: number): Promise<void> {
    try {
      image.rating = await api.setRating(image.id, rating);
    } catch (err) {
      this.error = String(err);
    }
  }

  clearMessages(): void {
    this.error = null;
    this.notice = null;
  }
}

export const gallery = new GalleryStore();

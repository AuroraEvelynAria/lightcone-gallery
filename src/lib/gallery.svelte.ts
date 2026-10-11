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
  Orientation,
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
  | { kind: "tag"; id: number }
  /**
   * 按画师筛选。画师从路径反推（见 `crate::artists`），不需要联网。
   * `name` 只用于显示 —— 分组永远按 `uid`，因为同一个 uid 可能有多个历史名字。
   */
  | { kind: "artist"; uid: string; name: string }
  /**
   * 按 Pixiv 官方标签筛选。标签来自联网补全的元数据，与主人自己打的标签
   * （`kind: "tag"`，挂在 `image_tags` 上）是两套东西，不要混。
   */
  | { kind: "pixivTag"; tag: string }
  /**
   * 作品库（PID 维度）。它不是 `images` 表的筛选条件，而是把内容区整个
   * 换成作品库视图 —— 与「全部图片 / 收藏」同款的侧栏原地切换。
   */
  | { kind: "artworks" };

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
  /** 画幅筛选。`null` = 不限。 */
  orientation = $state<Orientation | null>(null);

  scanning = $state(false);
  scanProgress = $state<ScanProgress | null>(null);

  error = $state<string | null>(null);
  notice = $state<string | null>(null);

  #unlisten: UnlistenFn | null = null;

  /**
   * 列表请求的代际。`refreshImages` 每次开跑时 +1，在途响应落地前核对，
   * 对不上就整体丢弃 —— 快速切换筛选时旧筛选的响应后到，会把新列表连
   * `total` 一起覆盖掉。`artworks.svelte.ts` 的 `#generation` 是同一道闸。
   */
  #generation = 0;

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
      case "artist":
        return `画师：${selection.name}`;
      case "pixivTag":
        return `标签：${selection.tag}`;
      case "artworks":
        return "作品库";
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
      orientation: this.orientation,
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
      case "artist":
        return { ...base, artistUids: [selection.uid] };
      case "pixivTag":
        return { ...base, pixivTags: [selection.tag] };
      case "favorites":
        return { ...base, favoritesOnly: true };
      default:
        return base;
    }
  }

  async refreshImages(): Promise<void> {
    const generation = ++this.#generation;
    this.loading = true;
    this.error = null;
    try {
      const page = await api.listImages(this.#query(0));
      // 已被更新的请求取代（比如用户又点了一个筛选）：这份响应过期，整体丢弃，
      // 连 error / loading 都不能碰 —— 那两个标志现在归最新的请求管。
      if (generation !== this.#generation) return;
      this.images = page.items;
      this.total = page.total;
    } catch (err) {
      if (generation === this.#generation) this.error = String(err);
    } finally {
      if (generation === this.#generation) this.loading = false;
    }
  }

  async loadMore(): Promise<void> {
    if (this.loading || this.loadingMore || !this.hasMore) return;
    // 不自增代际：loadMore 是当前这轮列表的延续。但落地前要核对 —— 在途期间
    // 若来了一轮新的 refreshImages，本页的偏移量已经失效，拼上去会把旧筛选的
    // 条目混进新列表。
    const generation = this.#generation;
    this.loadingMore = true;
    try {
      const page = await api.listImages(this.#query(this.images.length));
      if (generation !== this.#generation) return;
      this.images = [...this.images, ...page.items];
      this.total = page.total;
    } catch (err) {
      if (generation === this.#generation) this.error = String(err);
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

  /** 切换筛选维度会自动重新拉取列表。作品库不查 `images` 表，是个例外。 */
  select(selection: Selection): void {
    this.selection = selection;
    if (selection.kind !== "artworks") void this.refreshImages();
  }

  /** 桌面：系统目录对话框选图库根目录。Android 上走 Sidebar 的自绘选目录面板
   * （SAF 的 DocumentsUI 浏览大目录会崩，见 addLibraryAt）。 */
  async addLibrary(): Promise<void> {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({
      directory: true,
      multiple: false,
      title: "选择图库根目录",
    });
    if (typeof picked !== "string") return;
    await this.addLibraryAt(picked);
  }

  /** 登记目录并立即扫描 —— 桌面对话框与 Android 选目录面板共用的后半段。 */
  async addLibraryAt(path: string): Promise<void> {
    try {
      const library = await api.addLibrary(path);
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

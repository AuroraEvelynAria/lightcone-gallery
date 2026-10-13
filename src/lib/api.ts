/**
 * Tauri 命令的类型化封装。所有 IPC 调用都收口在这里，组件不直接 invoke。
 *
 * 约定：Rust 命令名用 snake_case，参数用 camelCase（Tauri 会自动转换命名风格）。
 */

import { invoke as tauriInvoke } from "@tauri-apps/api/core";

import type {
  Album,
  Artwork,
  ArtworkAspect,
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
  RatingCounts,
  ScanProgress,
  ScanSummary,
  SortBy,
  Stats,
  Tag,
  TagCount,
  TagGallery,
  ThumbnailProgress,
} from "./types";
// 常量要按值导入 —— `RATING_ANY` 会被当默认值用，`import type` 拿不到它
import { RATING_ANY } from "./types";

/** 扫描进度事件的频道名，与 Rust 端 `SCAN_PROGRESS_EVENT` 保持一致。 */
export const SCAN_PROGRESS_EVENT = "scan://progress";

/** 作品缩略图生成进度事件的频道名，与 Rust 端 `THUMBNAIL_PROGRESS_EVENT` 保持一致。 */
export const THUMBNAIL_PROGRESS_EVENT = "artworks://thumbnail-progress";

/** 元数据抓取进度事件的频道名，与 Rust 端 `META_PROGRESS_EVENT` 保持一致。 */
export const META_PROGRESS_EVENT = "pixiv://meta-progress";

export type { MetaProgress, ScanProgress, ThumbnailProgress };

/**
 * 冷启动竞态：Android 上 WebView 的 JS 会跑在 Rust `setup()` 托管 `AppState` 之前，
 * 这时任何需要 `State` 的命令都会抛
 * 「state not managed for field `state` on command `xxx`. You must call `.manage()`…」。
 *
 * 这不是调用方的错，等一下再试就好 —— 真机实测：**热重载页面一切正常，只有冷启动
 * 会撞上**（冷启动时外观设置读不回来、界面上挂一条报错，就是它）。
 *
 * 只对这**一个**错误做有界重试，其余错误原样抛出：真正的故障不该被拖成两秒超时。
 */
const STATE_RACE_MARKER = "state not managed";
const STATE_RACE_ATTEMPTS = 15;
const STATE_RACE_DELAY_MS = 120;

/**
 * 与 `invoke` 同签名，只多一层冷启动重试。
 *
 * 故意沿用 `invoke` 这个名字：下面五十多处调用点一个字都不用改，
 * 而「所有 IPC 都收口在这个文件」的约定也就自然保住了。
 */
async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  for (let attempt = 1; ; attempt++) {
    try {
      return await tauriInvoke<T>(cmd, args);
    } catch (err) {
      if (attempt >= STATE_RACE_ATTEMPTS || !String(err).includes(STATE_RACE_MARKER)) {
        throw err;
      }
      await new Promise((resolve) => setTimeout(resolve, STATE_RACE_DELAY_MS));
    }
  }
}

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
  /**
   * 退出应用。Android 全面屏返回手势的最后一站：MainActivity 把系统返回
   * 转发成 `lc-android-back`，前端的关闭栈（弹窗 → 抽屉 → …）都空了才调这里。
   * 桌面上没有返回手势，永远不会走到。
   */
  exitApp: () => invoke<void>("exit_app"),

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

  /** Android 沉浸式全屏：隐藏 / 恢复状态栏与手势条（检视弹窗的全屏按钮）。 */
  androidImmersive: (enabled: boolean) =>
    invoke<void>("android_immersive", { enabled }),
  /**
   * Android 前台服务开关：元数据补全期间挂通知栏常驻服务，豁免「应用后台
   * 断网」的系统省电策略（不挂的话主人一切走应用，网络立刻被掐）。
   */
  androidKeepAlive: (enabled: boolean) =>
    invoke<void>("android_keep_alive", { enabled }),
  /**
   * 列出一个目录的子目录（各带直属图片数）与它直属的图片数。
   * 实时 readdir 文件系统 —— 比 MediaStore 的索引准确（那个滞后且波动）。
   * Android 上传 `/storage/emulated/0/...` 形式的路径。
   */
  listSubdirs: (path: string) =>
    invoke<{ subdirs: { name: string; imageCount: number }[]; directImages: number }>(
      "android_list_subdirs",
      { path },
    ),
  /**
   * Android 系统目录选择器（SAF）。返回选中的物理路径（取消为空串），
   * 之后走与桌面相同的 add_library → walkdir 管线。
   */
  androidPickDirectory: () => invoke<string>("android_pick_directory"),

  // ---- 作品（阶段一：Pixiv 维度）------------------------------------------
  /** 扫描一个本地文件夹，解析 `{pid}_p{num}` 入库，并为缺缩略图的作品生成 WebP */
  scanArtworksFolder: (path: string) =>
    invoke<ArtworkScanSummary>("scan_artworks_folder", { path }),
  /**
   * Android：MediaStore 桥扫描。pixez 下到哪索引到哪——桥枚举全机图片里
   * 符合 `{pid}_p{n}` 命名的条目入库并生成缩略图，不需要选目录。
   */
  androidScanMediaStore: () =>
    invoke<ArtworkScanSummary>("android_scan_media_store"),
  /**
   * 分页读取作品列表：从第 offset 条开始返回 limit 条。
   *
   * 每条都带着 `originalPath` 与 `thumbnailPath`，卡片直接用后者渲染，无需额外 IPC。
   */
  listArtworks: (
    offset = 0,
    limit = 60,
    filters?: { artistUids?: string[]; pixivTags?: string[]; rating?: number },
  ) =>
    invoke<ArtworkPage>("list_artworks", {
      offset,
      limit,
      artistUids: filters?.artistUids ?? [],
      pixivTags: filters?.pixivTags ?? [],
      // -1 = 不限；0 = 全年龄；1 = R-18 及以上。默认值必须显式传，
      // 后端把缺省当成「不限」只是为了兼容旧调用
      rating: filters?.rating ?? RATING_ANY,
    }),
  /**
   * 读出当前筛选下**每一张**作品的宽高比，供瀑布流排版。
   *
   * 刻意不分页：瀑布流的总高 = 最高那一列有多高，拿不到全量就撑不准滚动条。
   * 只送 id 与比例时 4430 条约 70KB，一次取完最省事。
   *
   * **首次调用会慢一点**：老库的宽高要现场量（读缩略图文件头，并行），量完写回库，
   * 之后纯查表。量文件头在数据库锁之外跑，不会卡住同期的列表查询。
   */
  listArtworkAspects: (filters?: {
    artistUids?: string[];
    pixivTags?: string[];
    rating?: number;
  }) =>
    invoke<ArtworkAspect[]>("list_artwork_aspects", {
      artistUids: filters?.artistUids ?? [],
      pixivTags: filters?.pixivTags ?? [],
      rating: filters?.rating ?? RATING_ANY,
    }),
  // ---- 标签图库 -----------------------------------------------------------
  /**
   * 读出全部**自建**的标签图库，每条带实时命中的作品条数。
   *
   * 侧栏里内置的「全年龄 / R-18」两条不在这里 —— 它们就是 `rating` 一个维度，
   * 由前端直接拼出来，不落库。
   */
  listTagGalleries: () => invoke<TagGallery[]>("list_tag_galleries"),
  /** 新建一条。名字重复、标签与分级同时为空都会被后端拒绝并给出原因。 */
  createTagGallery: (name: string, tags: string[], rating: number) =>
    invoke<TagGallery>("create_tag_gallery", { name, tags, rating }),
  /** 删掉一条。只是去掉侧栏入口，**作品一条都不动**。 */
  deleteTagGallery: (id: number) => invoke<void>("delete_tag_gallery", { id }),
  /** 数一数各分级下有多少条作品（侧栏两条内置图库的计数 + 「未抓元数据」提示）。 */
  countArtworksByRating: () => invoke<RatingCounts>("count_artworks_by_rating"),

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
  /**
   * 自动补全：一批接一批跑到没有待补全的为止。
   *
   * **不会比手动点更激进** —— 同样的 800ms 间隔、非 200 不重试、撞限流立刻整轮停，
   * 只是多了「每 200 条歇 30 秒」的规律停顿，以及随时可停。
   * 进度通过 `META_PROGRESS_EVENT` 推送（带整轮的 overallDone / overallTotal）。
   */
  fetchAllMeta: () => invoke<MetaBatch>("fetch_all_meta"),
  /**
   * 把一张原图写进系统剪贴板。
   *
   * 为什么不用 WebView2 右键菜单里的「复制图片」：它靠图片 URL 去取字节，
   * 而我们的图走自定义 `asset://` 协议，它取不到，点了没反应。
   */
  copyImage: (path: string) => invoke<void>("copy_image_to_clipboard", { path }),
  /**
   * 把一张图设为**桌面壁纸**。
   *
   * 两端实现完全不同：Windows 走 `SystemParametersInfoW`，Android 交给
   * Kotlin 的 `WallpaperManager`。`target` **只对 Android 有意义**
   * （`home` 默认 / `lock` / `both`）—— Windows 没有「锁屏壁纸」这个概念。
   *
   * 失败原因要**原样显示给用户**：格式不被系统接受、ROM 不给设锁屏，
   * 这两种在两端都会遇到，静默失败会让人以为功能坏了。
   */
  setWallpaper: (path: string, target?: "home" | "lock" | "both") =>
    invoke<void>("set_wallpaper", { path, target: target ?? null }),
  /** 叫停自动补全。最多再抓完当前这批就停。返回当时是否在跑。 */
  stopMetaFetch: () => invoke<boolean>("stop_meta_fetch"),
  /** 按标签统计，倒序。标签只能联网取到。 */
  listPixivTags: (limit?: number) =>
    invoke<TagCount[]>("list_pixiv_tags", { limit: limit ?? null }),
  /** 元数据覆盖情况（已补全 / 已删除 / 失败 / 待补全）。 */
  pixivMetaStats: () => invoke<MetaStats>("pixiv_meta_stats"),
  /**
   * 把「已删除」标记清掉，让它们重新进入待补全。
   *
   * Pixiv 对「删除了」和「转私密了」都返回 404，无法区分；私密作品以后可能放回来，
   * 所以终态不该是单向门。返回清掉的条数。
   */
  resetMissingMeta: () => invoke<number>("reset_missing_meta"),
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

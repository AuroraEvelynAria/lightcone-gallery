<script lang="ts">
  /**
   * 作品库（Pixiv / PID 维度）视图。
   *
   * 以前是独立路由（/artworks），2026-10-11 起并入主界面：侧栏点「作品库 (PID)」
   * 原地切换到本视图，与「全部图片 / 收藏」同一套交互 —— 侧栏负责导航，本组件
   * 只负责内容区，顶部一栏与图库工具条同高同底色。
   *
   * 列表用虚拟滚动渲染，缩略图由扫描阶段预生成、地址随列表一起返回，
   * 卡片直接就能画出来。原图只在点击后的检视弹窗里加载。
   */
  import { onMount } from "svelte";
  import { fly, slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";

  import { listen, type UnlistenFn } from "@tauri-apps/api/event";

  import { api, META_PROGRESS_EVENT, THUMBNAIL_PROGRESS_EVENT } from "$lib/api";
  import { artworks, type ArtworkLayout } from "$lib/artworks.svelte";
  import { registerBackCloser } from "$lib/backStack.svelte";
  import { gallery } from "$lib/gallery.svelte";
  import { RATING_ALL_AGES, RATING_ANY, RATING_NSFW, type FilterForm } from "$lib/types";
  import ArtworkCard from "$lib/components/ArtworkCard.svelte";
  import ArtworkLightbox from "$lib/components/ArtworkLightbox.svelte";
  import VirtualGrid from "$lib/components/VirtualGrid.svelte";
  import FilterPanel from "$lib/components/FilterPanel.svelte";
  import VirtualMasonry from "$lib/components/VirtualMasonry.svelte";
  import type {
    ArtistSummary,
    Artwork,
    ArtworkScanSummary,
    MetaBatch,
    MetaProgress,
    MetaStats,
    TagCount,
    ThumbnailProgress,
  } from "$lib/types";

  let {
    /** 手机上的汉堡按钮回调（拉出侧栏抽屉） */
    onnav,
  }: {
    onnav?: () => void;
  } = $props();

  let scanning = $state(false);
  let summary = $state<ArtworkScanSummary | null>(null);
  let thumbProgress = $state<ThumbnailProgress | null>(null);
  let selected = $state<Artwork | null>(null);
  /** 弹窗当前这张在库里的序号，翻页要用。 */
  let selectedIndex = $state<number | null>(null);
  /** 忽略名单条数：被移出过、重扫也不会再收进来的作品。 */
  let ignoredCount = $state(0);
  /** 清空忽略名单的二次确认，与移除按钮同一套手势。 */
  let confirmingClear = $state(false);
  /**
   * 筛选面板是否展开。两端同一套：默认收起（纵向空间还给图片网格），展开时在
   * header 下方原地长出面板。手机和桌面不再各走一套（那边是常驻两行、这边是
   * 整屏右侧抽屉），筛选逻辑本来就是同一份。
   */
  let filterOpen = $state(false);

  $effect(() => {
    if (!filterOpen) return;
    return registerBackCloser(() => (filterOpen = false));
  });

  // 选择模式也算一层浮层：按返回先退出选择，而不是把整个作品库也退掉
  $effect(() => {
    if (!selecting) return;
    return registerBackCloser(() => setSelecting(false));
  });

  /**
   * 当前是不是从侧栏的某条**标签图库**进来的。
   *
   * 作品库和标签图库共用这一个视图组件，靠标题区分：进了图库就显示图库名，
   * 否则显示「作品库（PID 维度）」。
   */
  const activeGallery = $derived(
    gallery.selection.kind === "tagGallery" ? gallery.selection : null,
  );

  /**
   * 「筛选」按钮角标上的条件数。
   *
   * **分级也要算进去**：只按分级筛（比如从「R-18」图库进来）时，前两项都是 0，
   * 早先角标会显示一个刺眼的「0」。
   */
  const filterCount = $derived(
    artworks.artistUids.length +
      artworks.pixivTags.length +
      (artworks.rating !== RATING_ANY ? 1 : 0),
  );

  /** 有没有生效中的筛选条件（画师任一 / 标签同时包含 / 分级）。 */
  const hasFilter = $derived(
    artworks.artistUids.length > 0 ||
      artworks.pixivTags.length > 0 ||
      artworks.rating !== RATING_ANY,
  );

  /** 分级三态。数据源是 `illust_meta.x_restrict`（Pixiv 官方分级），不是标签。 */
  const RATING_OPTIONS = [
    { value: RATING_ANY, label: "全部" },
    { value: RATING_ALL_AGES, label: "全年龄" },
    { value: RATING_NSFW, label: "R-18+" },
  ];

  /**
   * 能不能把当前条件存成图库。
   *
   * **只认标签 + 分级**：标签图库就是这个形状，画师筛选不进图库（一个作品只有
   * 一个画师，按画师分库和直接点侧栏的画师项没区别）。
   */
  const canSaveGallery = $derived(artworks.pixivTags.length > 0 || artworks.rating !== RATING_ANY);

  /** 「存成图库」的草稿状态。 */
  let savingGallery = $state(false);
  let galleryName = $state("");

  /**
   * 筛选面板的形态（底部浮层 / 右侧抽屉）。持久化，主人切一次就记住。
   *
   * 两种都留着是刻意的：人机功效哪种更顺手得自己用一遍才知道，
   * 面板头部就有开关，不用跑去设置页。
   */
  const FILTER_FORM_KEY = "lightcone.artworks.filterForm";

  function readFilterForm(): FilterForm {
    try {
      const stored = localStorage.getItem(FILTER_FORM_KEY);
      if (stored === "sheet" || stored === "drawer") return stored;
    } catch {
      // 隐私模式下 localStorage 会抛，按「没存过」处理
    }
    return "sheet";
  }

  let filterForm = $state<FilterForm>(readFilterForm());

  function setFilterForm(next: FilterForm): void {
    if (next === filterForm) return;
    filterForm = next;
    try {
      localStorage.setItem(FILTER_FORM_KEY, next);
    } catch {
      // 存不下就只在本次会话生效
    }
  }

  /**
   * 桌面上展开的维度（`null` = 都收起）。
   *
   * 桌面不是「一个筛选面板」，而是**一排维度入口 + 锚点浮层**（京东 PC 那种）：
   * 点哪个入口，哪个从按钮下方浮出来盖在列表上，**不推挤内容**。
   * 同一时刻只开一个。
   */
  type FilterEntry = "rating" | "artist" | "tag";

  let openEntry = $state<FilterEntry | null>(null);

  const FILTER_ENTRIES: { key: FilterEntry; label: string }[] = [
    { key: "rating", label: "分级" },
    { key: "artist", label: "画师" },
    { key: "tag", label: "标签" },
  ];

  /**
   * 排版切换时的淡入。
   *
   * 这里可以带位移：网格容器里没有 `fixed` 后代（筛选浮层、批量条、检视弹窗
   * 都是它的兄弟节点），`transform` 不会把谁带跑。位移给「换了个摆法」这件事
   * 一点方向感，比单纯淡入更像一次重排。
   */
  function swapIn(_node: HTMLElement, params: { duration?: number } = {}) {
    return {
      duration: params.duration ?? 220,
      easing: cubicOut,
      css: (t: number) => `opacity: ${t}; transform: translateY(${(1 - t) * 8}px);`,
    };
  }

  /**
   * 桌面锚点浮层的出场：**从按钮下方「长出来」**—— 缩放 + 淡入，原点在左上角
   * （面板自己的 `origin-top-left` 负责）。
   *
   * 不用 `fly`：浮层本来就该待在按钮正下方，位移是错的隐喻；缩放才是
   * 「从按钮里展开」的那一层意思。
   */
  function popIn(node: HTMLElement) {
    void node;
    return {
      duration: 160,
      easing: cubicOut,
      css: (t: number) => `opacity: ${t}; transform: scale(${0.96 + t * 0.04});`,
    };
  }

  /** 每个维度上挂的已选条数，收起来也看得见「这维筛了几条」。 */
  function entryCount(key: FilterEntry): number {
    if (key === "rating") return artworks.rating !== RATING_ANY ? 1 : 0;
    if (key === "artist") return artworks.artistUids.length;
    return artworks.pixivTags.length;
  }

  /** 侧栏「标签图库 +」发来的一次性信号：展开筛选面板。 */
  let seenOpenFilterRequest = $state(0);
  $effect(() => {
    if (artworks.openFilterRequest === seenOpenFilterRequest) return;
    seenOpenFilterRequest = artworks.openFilterRequest;
    filterOpen = true;
  });

  // ---- 归类：画师 / 标签 --------------------------------------------------
  let artists = $state<ArtistSummary[]>([]);
  let pixivTags = $state<TagCount[]>([]);
  /** 两个搜索框，用来在 1200+ 位画师 / 上百个标签里筛出候选 */
  let artistQuery = $state("");
  let tagQuery = $state("");
  /** 胶囊最多铺多少个 —— 1238 位画师全铺出来既慢又没法看 */
  const CHIP_LIMIT = 24;

  /** 后端已按作品数倒序，所以不搜索时取前 N 个就是「作品最多的 N 位」 */
  let visibleArtists = $derived.by(() => {
    const needle = artistQuery.trim().toLowerCase();
    const list = needle
      ? artists.filter(
          (a) => a.name.toLowerCase().includes(needle) || a.uid.includes(needle),
        )
      : artists;
    return list.slice(0, CHIP_LIMIT);
  });
  let visibleTags = $derived.by(() => {
    const needle = tagQuery.trim().toLowerCase();
    const list = needle
      ? pixivTags.filter((t) => t.tag.toLowerCase().includes(needle))
      : pixivTags;
    return list.slice(0, CHIP_LIMIT);
  });

  /** 筛选提示里显示画师名而不是 uid。 */
  function artistNames(): string {
    return artworks.artistUids
      .map((uid) => artists.find((a) => a.uid === uid)?.name ?? uid)
      .join(" / ");
  }

  /** 胶囊点击：在数组里加/去一个值。 */
  function toggle(list: string[], value: string): string[] {
    return list.includes(value) ? list.filter((item) => item !== value) : [...list, value];
  }

  // ---- 联网补全元数据 -----------------------------------------------------
  let metaStats = $state<MetaStats | null>(null);
  let metaProgress = $state<MetaProgress | null>(null);
  let metaBusy = $state(false);
  let lastBatch = $state<MetaBatch | null>(null);
  /** 代理地址。国内直连 pixiv 不通，必须填本地代理，如 http://127.0.0.1:7897 */
  let proxy = $state("");
  let proxySaved = $state(false);

  /**
   * 元数据设置（代理 / 补单批 / 重查已删除）是否展开。默认收起 ——
   * 这几项配一次就够，常驻几行白占屏（手机实测 13.4% 屏高）。
   */
  let metaOpen = $state(false);

  // ---- 自动补全 ----------------------------------------------------------
  let autoRunning = $state(false);
  /** 本轮的起点，用来按实测速率估剩余时间。 */
  let autoStartedAt = 0;
  /** 整轮进度。事件里的 overallDone 是「本批开始前」的数字，要加上本批已处理。 */
  let overall = $state({ done: 0, total: 0 });

  /**
   * 预计剩余时间。
   *
   * 用**实测速率**而不是写死「1 秒一条」：被限流、代理变慢、或者冷却期间，
   * 写死的估计会一直骗人。速率不准时宁可不显示（返回 null）。
   */
  let eta = $derived.by(() => {
    if (!autoRunning || overall.done === 0 || !autoStartedAt) return null;
    const elapsed = (Date.now() - autoStartedAt) / 1000;
    const rate = overall.done / elapsed;
    if (rate <= 0) return null;
    return Math.round(Math.max(0, overall.total - overall.done) / rate);
  });

  /** 选择模式：卡片上出现勾选框，点卡片是勾选而不是打开弹窗。 */
  let selecting = $state(false);
  let picked = $state<Set<number>>(new Set());
  let removing = $state(false);
  /** 批量移除的二次确认。 */
  let confirmingBatch = $state(false);

  let progress = $derived(
    artworks.total === 0
      ? "尚未索引任何作品"
      : `共 ${artworks.total} 条作品页 · 已载入 ${artworks.ready}`,
  );

  onMount(() => {
    void artworks.bootstrap();
    void refreshIgnored();
    void refreshTaxonomy();
    void loadProxy();

    let metaUnlisten: UnlistenFn | undefined;
    let metaDisposed = false;
    void listen<MetaProgress>(META_PROGRESS_EVENT, (event) => {
      metaProgress = event.payload;
      if (event.payload.overallTotal > 0) {
        overall = {
          done: event.payload.overallDone + event.payload.processed,
          total: event.payload.overallTotal,
        };
      }
    }).then((stop) => {
      if (metaDisposed) stop();
      else metaUnlisten = stop;
    });

    // 扫描时缩略图是批量预生成的，可能持续几十秒，得有进度可看
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen<ThumbnailProgress>(THUMBNAIL_PROGRESS_EVENT, (event) => {
      thumbProgress = event.payload;
    }).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    });

    return () => {
      disposed = true;
      unlisten?.();
      metaDisposed = true;
      metaUnlisten?.();
    };
  });

  /** 是否运行在 Android 上——扫描入口在手机上是 MediaStore 桥，没有目录选择 */
  const isAndroid = navigator.userAgent.includes("Android");

  /**
   * 网格单列最小宽度。
   *
   * **窄屏必须放宽到 150**：手机上可用宽只有约 333 CSS px，200 的最小值只排得下
   * 一列，卡片于是一列铺满 —— 而屏幕密度普遍到 3.25，格子的物理边长就是 1084px，
   * 缩略图（当时是 400px 长边、短边 267px）被放大四倍，肉眼可见地糊。
   * 150 能排下两列（格宽 160 CSS px），取值与通用图库的 `ImageGrid` 一致。
   *
   * 断点跟 Tailwind 的 `md` 对齐（768），免得网格与 `md:` 修饰的样式在两个尺度上打架。
   */
  const NARROW_QUERY = "(max-width: 767px)";

  /**
   * 可选的排版方式。网格是定高方格（整齐、裁成正方），瀑布流保留原图长宽比
   * （所见即所得，参考 pixez）。两者共用同一份数据与同一套列宽公式。
   */
  const LAYOUT_OPTIONS: { value: ArtworkLayout; label: string; hint: string }[] = [
    { value: "grid", label: "网格", hint: "等高方格，整齐；原图会被裁成正方" },
    { value: "masonry", label: "瀑布流", hint: "保留原图长宽比，不裁切（pixez 那种）" },
  ];

  /**
   * 切换排版时把「当前看的是第几条」记下来。
   *
   * 两种排版的几何完全不同，像素位置没法直接换算 —— 但**数据下标**可以。
   * 交给新组件用它自己的几何算出该滚到哪，用户就不会因为换个摆法被弹回顶部
   * （在几千条里翻到一半时，这一下很致命）。
   */
  let anchorIndex = $state(0);

  function currentAnchor(): number {
    const cells = document.querySelectorAll<HTMLElement>("[data-index]");
    let smallest = Number.POSITIVE_INFINITY;
    for (const cell of cells) {
      const index = Number(cell.dataset.index);
      if (Number.isFinite(index) && index < smallest) smallest = index;
    }
    return Number.isFinite(smallest) ? smallest : 0;
  }

  function switchLayout(next: ArtworkLayout): void {
    if (next === artworks.layout) return;
    anchorIndex = currentAnchor();
    artworks.setLayout(next);
    // 比例表是按需拉的：切过去才拉，网格模式下一条都不多取
    if (next === "masonry") void artworks.ensureAspects();
  }

  // 切到瀑布流（含冷启动时它就是当前排版的情况）就把比例表备好。
  // `ensureAspects` 自带去重，重复触发只是空转。
  $effect(() => {
    if (artworks.layout !== "masonry") return;
    if (artworks.aspectsReady) return;
    void artworks.ensureAspects();
  });

  let narrow = $state(false);
  $effect(() => {
    const query = window.matchMedia(NARROW_QUERY);
    narrow = query.matches;
    const onChange = (event: MediaQueryListEvent) => (narrow = event.matches);
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  });

  /** 选择文件夹 -> 扫描入库 + 批量生成缩略图 -> 作废旧块重新拉取。 */
  async function pickAndScan(): Promise<void> {
    // 安卓上没有「选目录」这回事：pixez 下到哪，MediaStore 桥索引到哪
    if (isAndroid) {
      await scanFromMediaStore();
      return;
    }
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({
      directory: true,
      multiple: false,
      title: "选择包含 {pid}_p{num}.jpg 的目录",
    });
    if (typeof picked !== "string") return;

    scanning = true;
    thumbProgress = null;
    try {
      summary = await api.scanArtworksFolder(picked);
      // 库里多了行，之前缓存的块偏移量全部作废
      artworks.reset();
      await artworks.bootstrap();
      await refreshIgnored();
      // 新扫进来的图会带出新画师、也让 pending 变大 —— 不刷新的话「自动补全」
      // 按钮还停在上一次补完后的 0 上，一直是灰的，点不动
      await refreshTaxonomy();
    } catch (err) {
      artworks.error = String(err);
    } finally {
      scanning = false;
      thumbProgress = null;
    }
  }

  /** 安卓扫描：MediaStore 枚举全机图片，按 `{pid}_p{n}` 命名入库。 */
  async function scanFromMediaStore(): Promise<void> {
    scanning = true;
    thumbProgress = null;
    try {
      summary = await api.androidScanMediaStore();
      artworks.reset();
      await artworks.bootstrap();
      await refreshIgnored();
      // 新扫进来的图会带出新画师、也让 pending 变大 —— 不刷新的话「自动补全」
      // 按钮还停在上一次补完后的 0 上，一直是灰的，点不动
      await refreshTaxonomy();
    } catch (err) {
      artworks.error = String(err);
    } finally {
      scanning = false;
      thumbProgress = null;
    }
  }

  /**
   * 拉画师 / 标签 / 元数据统计。
   *
   * **凡是对作品库有增删的地方都必须调它**：`metaStats.pending` 决定「自动补全」
   * 按钮是否可点，漏调一次按钮就会一直灰着 —— 主人补完一轮再扫描新图，界面上
   * 的 pending 还停在上一次的 0，按钮点不动（这是真机撞出来的）。
   * 代理地址不在这里读，免得把主人正在输入但没保存的内容冲掉。
   */
  async function refreshTaxonomy(): Promise<void> {
    try {
      const [artistList, tagList, stats] = await Promise.all([
        api.listArtists(),
        api.listPixivTags(500),
        api.pixivMetaStats(),
      ]);
      artists = artistList;
      pixivTags = tagList;
      metaStats = stats;
      // 侧栏「标签图库」的计数也一起吃这趟刷新 —— 扫描 / 移除之后条数变了，
      // 侧栏上那个数字不跟着变会让人以为筛选坏了
      void gallery.refreshTagGalleries();
    } catch (err) {
      artworks.error = String(err);
    }
  }

  /** 读回已保存的代理地址。只在挂载时调一次。 */
  async function loadProxy(): Promise<void> {
    try {
      const settings = await api.loadSettings();
      proxy = settings["pixiv_proxy"] ?? "";
    } catch (err) {
      artworks.error = String(err);
    }
  }

  async function saveProxy(): Promise<void> {
    try {
      await api.saveSettings({ pixiv_proxy: proxy.trim() });
      proxySaved = true;
      setTimeout(() => (proxySaved = false), 2500);
    } catch (err) {
      artworks.error = String(err);
    }
  }

  /**
   * 补一批元数据。
   *
   * 后端是串行 + 800ms 间隔的，一批 100 条大约要 1.5 分钟 —— 这是刻意的限流，
   * 不是慢。所以按钮上要显示进度，并且中途不允许再点。
   */
  async function fetchBatch(): Promise<void> {
    if (metaBusy) return;
    metaBusy = true;
    metaProgress = null;
    lastBatch = null;
    // Android 上挂前台服务：批抓要跑一两分钟，挂着系统才不掐后台网络
    if (isAndroid) await api.androidKeepAlive(true).catch(() => {});
    try {
      lastBatch = await api.fetchMissingMeta(100);
      await refreshTaxonomy();
    } catch (err) {
      artworks.error = String(err);
    } finally {
      metaBusy = false;
      metaProgress = null;
      if (isAndroid && !autoRunning) await api.androidKeepAlive(false).catch(() => {});
    }
  }

  /**
   * 一键补完剩下的全部。
   *
   * 后端是「一批接一批」跑的，限流参数和手动点完全一样（800ms 间隔、非 200 不重试、
   * 撞限流立刻停），只是多了每 200 条歇 30 秒的停顿，所以不会比手动点更激进。
   */
  async function startAuto(): Promise<void> {
    if (autoRunning) return;
    autoRunning = true;
    autoStartedAt = Date.now();
    overall = { done: 0, total: 0 };
    lastBatch = null;
    // Android 上挂前台服务：整轮要跑几十分钟，主人必然切走应用 —— 不挂的话
    // 系统直接掐断后台网络，全部请求超时（真机实测 netpolicy blocked=APP_BACKGROUND）
    if (isAndroid) await api.androidKeepAlive(true).catch(() => {});
    try {
      lastBatch = await api.fetchAllMeta();
      await refreshTaxonomy();
    } catch (err) {
      artworks.error = String(err);
    } finally {
      autoRunning = false;
      metaProgress = null;
      if (isAndroid) await api.androidKeepAlive(false).catch(() => {});
    }
  }

  async function stopAuto(): Promise<void> {
    try {
      await api.stopMetaFetch();
    } catch (err) {
      artworks.error = String(err);
    }
  }

  /** 把秒数说成人话。 */
  function formatEta(seconds: number): string {
    if (seconds < 60) return `${seconds} 秒`;
    const minutes = Math.round(seconds / 60);
    if (minutes < 60) return `${minutes} 分钟`;
    return `${Math.floor(minutes / 60)} 小时 ${minutes % 60} 分`;
  }

  /** 清掉「已删除」标记，让它们重新进入待补全。 */
  async function resetMissing(): Promise<void> {
    try {
      const cleared = await api.resetMissingMeta();
      await refreshTaxonomy();
      artworks.error = null;
      artworks.notice = `已把 ${cleared} 条「已删除」放回待补全，下次批量会重新尝试`;
    } catch (err) {
      artworks.error = String(err);
    }
  }

  /** 换筛选条件：作废块缓存并按新条件重拉。 */
  async function applyFilter(): Promise<void> {
    try {
      await artworks.setFilter({
        artistUids: artworks.artistUids,
        pixivTags: artworks.pixivTags,
        rating: artworks.rating,
      });
    } catch (err) {
      artworks.error = String(err);
    }
  }

  async function clearFilter(): Promise<void> {
    artworks.artistUids = [];
    artworks.pixivTags = [];
    artworks.rating = RATING_ANY;
    await applyFilter();
  }

  /**
   * 把当前的标签 + 分级存成一条侧栏图库。
   *
   * 画师筛选**不会**被存进去 —— 标签图库只认标签与分级（见 `canSaveGallery`）。
   */
  async function saveGallery(): Promise<void> {
    const name = galleryName.trim();
    if (!name) return;
    try {
      await api.createTagGallery(name, artworks.pixivTags, artworks.rating);
      await gallery.refreshTagGalleries();
      artworks.error = null;
      artworks.notice = `已存成图库「${name}」，侧栏「标签图库」里能找到`;
      savingGallery = false;
      galleryName = "";
    } catch (err) {
      artworks.error = String(err);
    }
  }

  async function refreshIgnored(): Promise<void> {
    try {
      ignoredCount = await api.countIgnoredArtworks();
    } catch (err) {
      artworks.error = String(err);
    }
  }

  /**
   * 清空忽略名单。
   *
   * 「移除」会顺带把 `(pid, page)` 记进忽略名单，重扫才不会把它们收回来。
   * 代价是误移之后光重扫没用，得先清名单 —— 所以这里必须留一个显眼的出口。
   */
  async function clearIgnored(): Promise<void> {
    try {
      const cleared = await api.clearIgnoredArtworks();
      await refreshIgnored();
      artworks.error = null;
      artworks.notice = `已清空忽略名单（${cleared} 条），重新扫描目录即可再次收录`;
    } catch (err) {
      artworks.error = String(err);
    } finally {
      confirmingClear = false;
    }
  }

  // 提示 4 秒后自动消失，免得一直挂在页头
  $effect(() => {
    if (!artworks.notice) return;
    const timer = setTimeout(() => (artworks.notice = null), 4000);
    return () => clearTimeout(timer);
  });

  function toggleSelect(artwork: Artwork): void {
    const next = new Set(picked);
    if (next.has(artwork.id)) next.delete(artwork.id);
    else next.add(artwork.id);
    picked = next;
  }

  /**
   * 弹窗里翻页。
   *
   * 目标可能落在还没取过的数据块上（虚拟滚动只保证可视窗口内的块），
   * 所以交给 `fetchAt` 去等 —— 等不到就原地不动，不切到空白。
   */
  async function navigate(delta: number): Promise<void> {
    const index = selectedIndex;
    if (index === null) return;
    const next = await artworks.fetchAt(index + delta);
    if (!next) return;
    selectedIndex = index + delta;
    selected = next;
  }

  function setSelecting(value: boolean): void {
    selecting = value;
    if (!value) {
      picked = new Set();
      confirmingBatch = false;
    }
  }

  /**
   * 把作品移出作品库。
   *
   * 只删数据库记录与缩略图缓存，**原图不动** —— 这是整个项目的硬约束。
   * 刷新时保留滚动位置，所以不必重置整个列表。
   */
  async function remove(ids: number[]): Promise<void> {
    if (ids.length === 0 || removing) return;
    removing = true;
    try {
      const removed = await api.removeArtworks(ids);
      artworks.refreshAfterRemoval(removed);
      picked = new Set();
      confirmingBatch = false;
      await refreshIgnored();
      // 移出去的作品不该再算进 pending，画师作品数也变了
      await refreshTaxonomy();
    } catch (err) {
      artworks.error = String(err);
    } finally {
      removing = false;
    }
  }
</script>

  <!-- 两种排版共用同一份格子内容：换排版只该换摆放方式，卡片本身一个字都不变 -->
  {#snippet artworkCell(index: number)}
    {@const artwork = artworks.at(index)}
    {#if artwork}
      <ArtworkCard
        {artwork}
        {selecting}
        selected={picked.has(artwork.id)}
        onopen={(item) => {
          selected = item;
          selectedIndex = index;
        }}
        onselecttoggle={toggleSelect}
        onremove={(item) => void remove([item.id])}
      />
    {:else}
      <div class="h-full w-full animate-pulse rounded-card bg-surface-850"></div>
    {/if}
  {/snippet}

  <!--
    三个维度各自成块：桌面把它们分别塞进三个**锚点浮层**，手机一次性全铺在面板里 ——
    同一份内容，不再两端各写一套（旧版就是这么漂移的）。

    刻意不用 <select>：画师有 1238 位、标签上百个，下拉里翻是不现实的，
    而且原生下拉在自动化里驱动不了（合成点击弹不开、setValue 只给焦点）。
    改成「搜索框 + 热门胶囊」：搜索把候选收窄，点胶囊即筛选 —— 点击是真实按钮，
    既能测也好用。
  -->

  {#snippet ratingGroup()}
    <!-- 分级：数据源是 Pixiv 官方分级（illust_meta.x_restrict），**不是标签** ——
         作者没打 R-18 标签的成人向作品也能被这条筛出来 -->
    <div class="flex flex-wrap items-center gap-2">
      <span class="w-full text-[11px] font-semibold tracking-wide text-muted uppercase">分级</span>
      <div class="flex flex-wrap gap-1.5">
        {#each RATING_OPTIONS as option (option.value)}
          <button
            type="button"
            class="cursor-pointer rounded-full border px-2.5 py-0.5 text-xs transition {artworks.rating ===
            option.value
              ? 'border-accent-500 bg-accent-500/20 text-accent-400'
              : 'border-surface-700 text-soft hover:border-surface-600 hover:text-fg'}"
            aria-pressed={artworks.rating === option.value}
            onclick={() => {
              artworks.rating = option.value;
              void applyFilter();
            }}
          >
            {option.label}
          </button>
        {/each}
      </div>
      {#if artworks.rating !== RATING_ANY && (gallery.ratingCounts?.unknown ?? 0) > 0}
        <!-- 没抓过元数据的作品两个分级都不属于，得说清楚，否则像丢了东西 -->
        <span class="text-[10px] text-mark">
          另有 {gallery.ratingCounts?.unknown} 条还没抓元数据，不计入
        </span>
      {/if}
    </div>
  {/snippet}

  {#snippet artistGroup()}
    <div class="flex flex-col gap-2">
      <span class="text-[11px] font-semibold tracking-wide text-muted uppercase">画师</span>
      <input
        bind:value={artistQuery}
        placeholder="搜画师名或 uid…"
        class="w-full rounded-md border border-surface-700 bg-surface-850 px-2 py-1 text-xs text-soft outline-none placeholder:text-mark focus:border-accent-500"
      />
      <div class="flex flex-wrap gap-1.5">
        {#each visibleArtists as artist (artist.uid)}
          <button
            type="button"
            class="cursor-pointer rounded-full border px-2.5 py-0.5 text-xs transition {artworks.artistUids.includes(
            artist.uid)
              ? 'border-accent-500 bg-accent-500/20 text-accent-400'
              : 'border-surface-700 text-soft hover:border-surface-600 hover:text-fg'}"
            aria-pressed={artworks.artistUids.includes(artist.uid)}
            onclick={() => {
              artworks.artistUids = toggle(artworks.artistUids, artist.uid);
              void applyFilter();
            }}
          >
            {artist.name} <span class="text-mark tabular-nums">{artist.artworks}</span>
          </button>
        {:else}
          <span class="text-xs text-muted">没有匹配的画师</span>
        {/each}
      </div>
      {#if artistQuery.trim() === "" && artists.length > CHIP_LIMIT}
        <span class="text-[10px] text-mark">（只列作品最多的 {CHIP_LIMIT} 位，搜名字找其他）</span>
      {/if}
    </div>
  {/snippet}

  {#snippet tagGroup()}
    <div class="flex flex-col gap-2">
      <span class="text-[11px] font-semibold tracking-wide text-muted uppercase">标签</span>
      <input
        bind:value={tagQuery}
        placeholder="搜标签…"
        class="w-full rounded-md border border-surface-700 bg-surface-850 px-2 py-1 text-xs text-soft outline-none placeholder:text-mark focus:border-accent-500"
      />
      <div class="flex flex-wrap gap-1.5">
        {#each visibleTags as tag (tag.tag)}
          <button
            type="button"
            class="cursor-pointer rounded-full border px-2.5 py-0.5 text-xs transition {artworks.pixivTags.includes(
            tag.tag)
              ? 'border-accent-500 bg-accent-500/20 text-accent-400'
              : 'border-surface-700 text-soft hover:border-surface-600 hover:text-fg'}"
            aria-pressed={artworks.pixivTags.includes(tag.tag)}
            onclick={() => {
              artworks.pixivTags = toggle(artworks.pixivTags, tag.tag);
              void applyFilter();
            }}
          >
            {tag.tag} <span class="text-mark tabular-nums">{tag.count}</span>
          </button>
        {:else}
          <span class="text-xs text-muted">
            {pixivTags.length === 0
              ? "还没有标签数据 —— 先在「元数据」里联网补一次"
              : "没有匹配的标签"}
          </span>
        {/each}
      </div>
      {#if tagQuery.trim() === "" && pixivTags.length > CHIP_LIMIT}
        <span class="text-[10px] text-mark">（只列最常见的 {CHIP_LIMIT} 个）</span>
      {/if}
    </div>
  {/snippet}

  {#snippet filterSummary()}
    {#if hasFilter}
      <div class="flex flex-wrap items-center gap-2">
        <span class="text-[11px] text-muted">
          已筛选：
          {#if artworks.artistUids.length > 0}
            画师 <span class="text-accent-400">{artistNames()}</span>
            <span class="text-mark">（任一）</span>
          {/if}
          {#if artworks.pixivTags.length > 0}
            标签 <span class="text-accent-400">{artworks.pixivTags.join(" + ")}</span>
            {#if artworks.pixivTags.length > 1}
              <span class="text-mark">（同时包含）</span>
            {/if}
          {/if}
          {#if artworks.rating !== RATING_ANY}
            分级
            <span class="text-accent-400">
              {RATING_OPTIONS.find((o) => o.value === artworks.rating)?.label ?? "?"}
            </span>
          {/if}
          · 共 <span class="tabular-nums text-soft">{artworks.total}</span> 条
        </span>
        <button
          type="button"
          class="cursor-pointer rounded-md border border-accent-500 bg-accent-500/15 px-2 py-0.5 text-xs text-accent-400 transition hover:bg-accent-500/25"
          onclick={clearFilter}
        >
          清除筛选
        </button>

        <!-- 存成侧栏里的一条独立图库。只认标签 + 分级（见 canSaveGallery） -->
        {#if !savingGallery}
          <button
            type="button"
            class="cursor-pointer rounded-md border border-surface-700 px-2 py-0.5 text-xs text-soft transition hover:border-surface-600 hover:text-fg disabled:cursor-not-allowed disabled:opacity-50"
            disabled={!canSaveGallery}
            title={canSaveGallery
              ? "把这组标签 + 分级存成侧栏里的一条独立图库"
              : "标签图库只记「标签 + 分级」，先选一个标签或分级"}
            onclick={() => {
              savingGallery = true;
              galleryName = "";
            }}
          >
            存成图库
          </button>
        {:else}
          <form
            class="flex items-center gap-1"
            onsubmit={(event) => {
              event.preventDefault();
              void saveGallery();
            }}
          >
            <input
              bind:value={galleryName}
              placeholder="图库名字"
              autocomplete="off"
              class="w-32 rounded-md border border-surface-700 bg-surface-850 px-2 py-0.5 text-xs text-fg outline-none placeholder:text-mark focus:border-accent-500"
              onkeydown={(event) => event.key === "Escape" && (savingGallery = false)}
            />
            <button
              type="submit"
              class="cursor-pointer rounded-md bg-accent-600 px-2 py-0.5 text-xs text-on-accent transition hover:bg-accent-500 disabled:opacity-50"
              disabled={!galleryName.trim()}
            >
              保存
            </button>
            <button
              type="button"
              class="cursor-pointer rounded-md border border-surface-700 px-2 py-0.5 text-xs text-soft transition hover:border-surface-600 hover:text-fg"
              onclick={() => (savingGallery = false)}
            >
              取消
            </button>
          </form>
        {/if}
      </div>
    {/if}
  {/snippet}

  <!-- 手机面板：三个维度一次全铺。浮层空间有限，不该再让人为了切维度多点一次 -->
  {#snippet filterBody()}
    <div class="flex flex-col gap-3">
      {@render ratingGroup()}
      {@render artistGroup()}
      {@render tagGroup()}
      {@render filterSummary()}
    </div>
  {/snippet}

<div class="flex h-full min-h-0 flex-1 flex-col text-fg">
  <!-- 与图库工具条同高同底色（h-14 / bg-surface-900 / px-4），观感上就是同一个壳；
       手机上允许换行 -->
  <header
    class="flex min-h-14 shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-b border-surface-800
           bg-surface-900 px-4 md:h-14 md:flex-nowrap md:gap-3"
  >
    {#if onnav}
      <button
        type="button"
        class="cursor-pointer rounded-md p-1.5 text-muted transition hover:bg-surface-850 hover:text-fg md:hidden"
        aria-label="打开导航"
        onclick={onnav}
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-5">
          <path stroke-linecap="round" stroke-linejoin="round" d="M3.75 6.75h16.5m-16.5 5.25h16.5m-16.5 5.25h16.5" />
        </svg>
      </button>
    {/if}
    <div class="min-w-0">
      <!-- 从侧栏的标签图库进来时显示图库名：两条入口共用这一个视图，
           标题不变的话根本分不清现在待在哪一条里（主人报过「串了」） -->
      <h1 class="truncate text-sm font-semibold text-fg">
        {activeGallery ? activeGallery.name : "作品库（PID 维度）"}
      </h1>
      <p class="truncate text-[11px] tabular-nums text-muted">
        {#if artworks.busy && artworks.ready === 0}
          正在读取…
        {:else}
          {progress}
        {/if}
      </p>
    </div>

    <div class="flex-1"></div>

    <!-- 排版切换：网格 / 瀑布流。两端都能切，选过的记在 localStorage 里；
         窄屏首次进来默认瀑布流（那正是这套排版存在的理由） -->
    <div class="flex shrink-0 items-center rounded-md border border-surface-700 p-0.5">
      {#each LAYOUT_OPTIONS as option (option.value)}
        <button
          type="button"
          class="cursor-pointer rounded px-2 py-1 text-xs transition {artworks.layout ===
          option.value
            ? 'bg-accent-600 font-medium text-on-accent'
            : 'text-soft hover:text-fg'}"
          aria-pressed={artworks.layout === option.value}
          title={option.hint}
          onclick={() => switchLayout(option.value)}
        >
          {option.label}
        </button>
      {/each}
    </div>

    <!-- 筛选开关（两端同一个）：收起把纵向空间还给网格，展开原地长出面板。
         按钮上挂已选条件数，收起时也能一眼看到筛了几条 -->
    <button
      type="button"
      class="relative cursor-pointer rounded-md border px-3 py-1.5 text-xs transition md:hidden
             {hasFilter || filterOpen
               ? 'border-accent-500 bg-accent-500/15 text-accent-400'
               : 'border-surface-700 text-soft'}"
      aria-expanded={filterOpen}
      onclick={() => (filterOpen = !filterOpen)}
    >
      筛选
      {#if hasFilter}
        <span
          class="absolute -top-1.5 -right-1.5 grid size-4 place-items-center rounded-full bg-accent-500 text-[10px] font-bold text-on-accent tabular-nums"
        >
          {filterCount}
        </span>
      {/if}
    </button>

    {#if artworks.total > 0}
      <button
        type="button"
        class="cursor-pointer rounded-md border px-3 py-1.5 text-xs transition {selecting
          ? 'border-accent-500 bg-accent-500/15 text-accent-400'
          : 'border-surface-700 text-soft hover:border-surface-600 hover:text-fg'}"
        aria-pressed={selecting}
        onclick={() => setSelecting(!selecting)}
      >
        {selecting ? "退出选择" : "选择"}
      </button>
    {/if}

    {#if ignoredCount > 0}
      <button
        type="button"
        class="cursor-pointer rounded-md border px-3 py-1.5 text-xs transition {confirmingClear
          ? 'border-rose-600 bg-rose-600/90 text-white'
          : 'border-surface-700 text-muted hover:border-surface-600 hover:text-fg'}"
        title="这些作品被移出过作品库，重新扫描目录也不会再收进来。清空后重扫即可再次收录。"
        onclick={() => {
          if (confirmingClear) void clearIgnored();
          else confirmingClear = true;
        }}
        onmouseleave={() => (confirmingClear = false)}
      >
        {confirmingClear ? `确认清空 ${ignoredCount} 条？` : `已忽略 ${ignoredCount} 条 · 清空`}
      </button>
    {/if}

    <button
      type="button"
      class="flex cursor-pointer items-center gap-1.5 rounded-md bg-accent-600 px-3 py-1.5 text-sm
             font-medium text-on-accent transition hover:bg-accent-500 disabled:cursor-not-allowed disabled:opacity-60"
      disabled={scanning}
      onclick={pickAndScan}
    >
      {#if scanning}
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          class="size-4 animate-spin"
        >
          <path stroke-linecap="round" d="M12 3a9 9 0 019 9" />
        </svg>
        {#if thumbProgress}
          生成缩略图 {thumbProgress.processed}/{thumbProgress.total}
        {:else}
          扫描中…
        {/if}
      {:else}
        {isAndroid ? "扫描手机里的 pixez 图" : "选择文件夹并扫描"}
      {/if}
    </button>
  </header>

  <!-- 归类筛选：画师从路径反推（不联网），标签来自联网补全的元数据 -->
  <!--
    桌面：**一排维度入口 + 锚点浮层**（京东 PC 那种）。点哪个入口，哪个从按钮下方
    浮出来盖在列表上，**不推挤内容**；同一时刻只开一个。整条栏只有一行，
    不像更早那版常驻两组胶囊那样一直吃纵向空间。

    手机：入口收进 header 的「筛选」按钮，面板是浮层（底部浮起 / 右侧滑出，可切），
    同样不推挤内容。两种形态的取舍见 `FilterPanel`。
  -->
  <div
    class="relative hidden shrink-0 items-center gap-2 border-b border-surface-800 bg-surface-900 px-4 py-1.5 md:flex"
  >
    {#each FILTER_ENTRIES as entry (entry.key)}
      {@const count = entryCount(entry.key)}
      <button
        type="button"
        class="cursor-pointer rounded-md border px-2.5 py-1 text-xs transition {openEntry ===
        entry.key
          ? 'border-accent-500 bg-accent-500/15 text-accent-400'
          : count > 0
            ? 'border-accent-500/60 text-accent-400'
            : 'border-surface-700 text-soft hover:border-surface-600 hover:text-fg'}"
        aria-expanded={openEntry === entry.key}
        onclick={() => (openEntry = openEntry === entry.key ? null : entry.key)}
      >
        {entry.label}
        {#if count > 0}
          <span class="ml-1 tabular-nums text-accent-400">{count}</span>
        {/if}
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.2"
          class="ml-1 inline size-3 transition-transform {openEntry === entry.key ? 'rotate-180' : ''}"
        >
          <path stroke-linecap="round" stroke-linejoin="round" d="M19.5 8.25l-7.5 7.5-7.5-7.5" />
        </svg>
      </button>
    {/each}

    <span class="mx-1 h-4 w-px shrink-0 bg-surface-800"></span>

    <!-- 摘要与「清除 / 存成图库」直接摆在这一行上，不占额外高度 -->
    <div class="flex min-w-0 flex-1 items-center gap-2">
      {@render filterSummary()}
    </div>

    <!-- 点面板外面就收起。刻意用透明背板而不是 document 监听：
         背板是真实节点，点击顺序确定，也不会漏掉「点到 iframe / 弹窗」那种边角。

         **起点从这一行的下边缘算**（`top-full` 而不是 `inset-0`）：盖满整屏的话，
         入口按钮自己也在这块背板底下，于是「开着一个维度再点另一个」会先被背板
         吃掉那一下、变成要连点两次 —— 而这一行正是最该一击即中的地方。 -->
    {#if openEntry}
      <button
        type="button"
        class="absolute inset-x-0 top-full z-20 h-screen cursor-default"
        aria-label="收起筛选"
        tabindex="-1"
        onclick={() => (openEntry = null)}
      ></button>
      <div
        class="absolute top-full left-2 z-30 w-[min(560px,80vw)] origin-top-left rounded-card border
               border-surface-700 floating-panel p-3 shadow-2xl"
        transition:popIn
        role="dialog"
        aria-label={`筛选：${FILTER_ENTRIES.find((e) => e.key === openEntry)?.label ?? ""}`}
      >
        {#if openEntry === "rating"}
          {@render ratingGroup()}
        {:else if openEntry === "artist"}
          {@render artistGroup()}
        {:else}
          {@render tagGroup()}
        {/if}
      </div>
    {/if}
  </div>

  <!-- 手机：条件摘要常驻一行（不展开面板也看得见「现在筛了什么」） -->
  {#if hasFilter}
    <div
      class="flex shrink-0 items-center gap-2 border-b border-surface-800 bg-surface-900 px-4 py-1.5 text-[11px] text-muted md:hidden"
    >
      <span class="min-w-0 truncate">
        已筛选：
        {#if artworks.artistUids.length > 0}
          画师 <span class="text-accent-400">{artistNames()}</span>
        {/if}
        {#if artworks.pixivTags.length > 0}
          标签 <span class="text-accent-400">{artworks.pixivTags.join(" + ")}</span>
        {/if}
        {#if artworks.rating !== RATING_ANY}
          分级
          <span class="text-accent-400">
            {RATING_OPTIONS.find((option) => option.value === artworks.rating)?.label ?? "?"}
          </span>
        {/if}
        · 共 <span class="tabular-nums text-soft">{artworks.total}</span> 条
      </span>
      <button
        type="button"
        class="shrink-0 cursor-pointer rounded-md border border-accent-500 bg-accent-500/15 px-2 py-0.5 text-xs text-accent-400 transition hover:bg-accent-500/25"
        onclick={clearFilter}
      >
        清除筛选
      </button>
    </div>
  {/if}

  {#if filterOpen}
    <FilterPanel form={filterForm} onclose={() => (filterOpen = false)} onform={setFilterForm}>
      {@render filterBody()}
    </FilterPanel>
  {/if}

  <!-- 联网补全元数据：标题 / 画师原名 / 标签 -->
  <!--
    **默认收起**。这几行（统计 / 代理输入 / 保存代理 + 补全 100 条 / 重查已删除）
    全是「配一次就再也不碰」的东西，但手机上实测它一个人吃掉 **13.4% 的屏高**
    （112 CSS / 835 CSS），比筛选那一整行还多。

    收起后只剩一行：现状数字 + **主操作常驻**（自动补全 / 停止）——
    补全是一等动作，不该跟着一起藏起来。要改代理或补单批再点箭头展开。
  -->
  <div class="shrink-0 border-b border-surface-800 bg-surface-850">
    <div class="flex flex-wrap items-center gap-2 px-4 py-1.5 text-[11px] text-muted">
      <button
        type="button"
        class="flex cursor-pointer items-center gap-1 rounded px-1 py-0.5 transition hover:bg-surface-800 hover:text-fg"
        aria-expanded={metaOpen}
        title={metaOpen ? "收起元数据设置" : "展开元数据设置（代理 / 补单批 / 重查已删除）"}
        onclick={() => (metaOpen = !metaOpen)}
      >
        <span>元数据</span>
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.2"
          class="size-3 transition-transform duration-200 {metaOpen ? 'rotate-180' : ''}"
        >
          <path stroke-linecap="round" stroke-linejoin="round" d="M19.5 8.25l-7.5 7.5-7.5-7.5" />
        </svg>
      </button>

      {#if metaStats}
        <span class="tabular-nums">
          已补全 <span class="font-semibold text-accent-400">{metaStats.done}</span>
          {#if metaStats.missing > 0}
            · Pixiv 已删除
            <span
              class="text-muted"
              title="这个作品在 Pixiv 上返回 404（作者删除 / 转私密 / 下架）。**本地文件完全没动**，只是 Pixiv 那一侧没了。这是终态，不再重抓。"
            >
              {metaStats.missing}
            </span>
          {/if}
          {#if metaStats.failed > 0}
            · 失败 <span class="text-amber-400">{metaStats.failed}</span>
          {/if}
          · 待补全 <span class="font-semibold text-soft">{metaStats.pending}</span>
        </span>
      {:else}
        <span>读取中…</span>
      {/if}

      <!-- 主操作常驻：一轮跑几十分钟，收起状态下也必须看得见进度、点得到停止 -->
      {#if autoRunning}
        <button
          type="button"
          class="cursor-pointer rounded-md border border-rose-600 bg-rose-600/90 px-3 py-1 text-xs font-medium text-white transition hover:bg-rose-500"
          onclick={stopAuto}
        >
          停止
        </button>
      {:else}
        <button
          type="button"
          class="cursor-pointer rounded-md bg-accent-600 px-3 py-1 text-xs font-medium text-on-accent transition hover:bg-accent-500 disabled:cursor-not-allowed disabled:opacity-60"
          disabled={metaBusy || metaStats?.pending === 0}
          onclick={startAuto}
          title="一批接一批跑完剩下的全部。限流参数和手动点一样，另外每 200 条歇 30 秒。"
        >
          {#if metaStats?.pending === 0}
            已全部补全
          {:else}
            自动补全 {metaStats?.pending ?? 0} 条
          {/if}
        </button>
      {/if}

      {#if autoRunning && overall.total > 0}
        <!-- 整轮进度条。收起状态下也留着 —— 它一跑就是几十分钟 -->
        <div class="flex min-w-40 flex-1 items-center gap-2">
          <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-surface-700">
            <div
              class="h-full rounded-full bg-accent-500 transition-[width] duration-300"
              style="width: {Math.min(100, (overall.done * 100) / overall.total)}%"
            ></div>
          </div>
          <span class="shrink-0 tabular-nums text-soft">{overall.done}/{overall.total}</span>
          {#if eta !== null}
            <span class="shrink-0 tabular-nums text-mark">还需约 {formatEta(eta)}</span>
          {/if}
        </div>
      {/if}
    </div>

    {#if metaOpen}
      <div
        class="flex flex-wrap items-center gap-2 border-t border-surface-800/70 px-4 py-2 text-[11px] text-muted"
        transition:slide={{ duration: 200 }}
      >
        <input
          bind:value={proxy}
          placeholder="代理，如 http://127.0.0.1:7897（国内直连不通）"
          class="w-72 rounded-md border border-surface-700 bg-surface-900 px-2 py-1 text-xs text-soft outline-none placeholder:text-mark focus:border-accent-500"
        />
        <button
          type="button"
          class="cursor-pointer rounded-md border border-surface-700 px-2 py-1 text-xs text-soft transition hover:border-surface-600 hover:text-fg"
          onclick={saveProxy}
        >
          {proxySaved ? "已保存" : "保存代理"}
        </button>

        <button
          type="button"
          class="cursor-pointer rounded-md border border-surface-700 px-3 py-1 text-xs text-soft transition hover:border-surface-600 hover:text-fg disabled:cursor-not-allowed disabled:opacity-60"
          disabled={metaBusy || metaStats?.pending === 0}
          onclick={fetchBatch}
          title="只补一批 100 条。限流参数与自动补全完全一样（800ms 间隔、非 200 不重试、撞限流立刻停）。"
        >
          {#if metaBusy && metaProgress}
            抓取中 {metaProgress.processed}/{metaProgress.total}
          {:else if metaBusy}
            抓取中…
          {:else}
            补全 100 条
          {/if}
        </button>

        {#if metaStats && metaStats.missing > 0}
          <button
            type="button"
            class="cursor-pointer rounded-md border border-surface-700 px-2 py-0.5 text-[11px] text-muted transition hover:border-surface-600 hover:text-fg"
            title="Pixiv 对「删除了」和「转私密了」都返回 404，无法区分。私密作品以后可能放回来，所以这里可以清掉标记让它们重新进入待补全。本地文件不受影响。"
            onclick={resetMissing}
          >
            重查 Pixiv 上已删除的 {metaStats.missing} 条
          </button>
        {/if}

        {#if lastBatch}
          <span class="tabular-nums">
            本批：成功 <span class="text-accent-400">{lastBatch.fetched}</span>
            · Pixiv 已删除 {lastBatch.missing}
            {#if lastBatch.failed > 0}
              · 失败 <span class="text-amber-400">{lastBatch.failed}</span>
            {/if}
            {#if lastBatch.stopped}
              <span class="text-mark">· 已停止（进度已保存，再点一次接着补）</span>
            {:else if lastBatch.rateLimited}
              <span class="text-rose-400">
                · 被 Pixiv 限流，已立刻停止 —— 过一阵再补，别连续点
              </span>
            {:else if lastBatch.aborted}
              <span class="text-rose-400">· 连续失败过多已中止，检查代理是否开着</span>
            {/if}
            {#if lastBatch.lastError}
              <!-- 最后一条失败的原始原因：代理断 / 超时 / TLS，别让主人对着数字猜 -->
              <span class="text-rose-400">（{lastBatch.lastError}）</span>
            {/if}
          </span>
        {/if}
      </div>
    {/if}
  </div>

  {#if summary}
    <div class="border-b border-surface-800 bg-surface-850 px-4 py-2 text-[11px] text-muted">
      上次扫描：发现 {summary.discovered} 个图片 · 命名匹配 {summary.matched} · 新入库
      <span class="text-accent-400">{summary.inserted}</span>
      · 去重跳过 <span class="text-amber-400">{summary.duplicates}</span>
      · 命名不符跳过 {summary.skipped}
      {#if summary.ignored > 0}
        · 已忽略跳过 <span class="text-accent-400">{summary.ignored}</span>
      {/if}
      · 缩略图新生成 <span class="text-accent-400">{summary.thumbnailsGenerated}</span>
      / 复用 {summary.thumbnailsSkipped}
      {#if summary.thumbnailsFailed > 0}
        · 失败 <span class="text-rose-400">{summary.thumbnailsFailed}</span>
      {/if}
    </div>
  {/if}

  {#if artworks.error}
    <div class="mx-4 mt-3 rounded-md border border-rose-900/60 bg-rose-950/60 px-4 py-2.5 text-sm text-rose-200">
      {artworks.error}
    </div>
  {/if}

  {#if artworks.notice}
    <div
      class="border-b border-surface-800 bg-accent-500/10 px-4 py-2 text-[11px] text-accent-400"
    >
      {artworks.notice}
    </div>
  {/if}

  <main class="min-h-0 flex-1">
    {#if !artworks.booted}
      <!-- 首块还没到，先铺一层骨架，避免闪一下「库是空的」。
           列数与下面的 VirtualGrid 对齐：窄屏 150、宽屏 200，否则首屏与真内容会跳一下 -->
      <div
        class="grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-3 p-4
               md:grid-cols-[repeat(auto-fill,minmax(200px,1fr))]"
      >
        {#each Array.from({ length: 12 }) as _, index (index)}
          <div class="aspect-square animate-pulse rounded-card bg-surface-850"></div>
        {/each}
      </div>
    {:else if artworks.total === 0}
      <div class="flex flex-col items-center justify-center gap-2 py-24 text-center">
        <p class="text-sm text-muted">作品库还是空的。</p>
        <p class="text-xs text-muted">
          {#if isAndroid}
            点上方「扫描手机里的 pixez 图」，全机符合
            <code class="text-muted">{'{pid}_p{num}.jpg'}</code> 命名的图片
            会自动入库。
          {:else}
            点上方「选择文件夹并扫描」，选一个含有 <code class="text-muted"
              >{'{pid}_p{num}.jpg'}</code
            > 文件的目录。
          {/if}
        </p>
      </div>
    {:else if artworks.layout === "masonry" && !artworks.aspectsReady}
      <!-- 比例表还没到位就不能排版：瀑布流的总高取决于全部比例，缺一条都撑不准
           滚动条。第一次切过来要现场量一遍老库的尺寸，所以这里必须说清楚在等什么 -->
      <div class="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
        {#if artworks.aspectsBusy}
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-8 animate-spin text-accent-500">
            <path stroke-linecap="round" d="M12 3a9 9 0 019 9" />
          </svg>
          <p class="text-sm text-muted">正在准备瀑布流布局…</p>
          <p class="text-xs text-mark">第一次切过来要量一遍原图尺寸，只此一次，之后就快了</p>
        {:else}
          <p class="text-sm text-muted">瀑布流布局没能就绪。</p>
          <button
            type="button"
            class="cursor-pointer rounded-md border border-surface-700 px-3 py-1 text-xs text-soft transition hover:border-surface-600 hover:text-fg"
            onclick={() => void artworks.ensureAspects()}
          >
            重试
          </button>
        {/if}
      </div>
    {:else if artworks.layout === "masonry"}
      <div class="h-full" in:swapIn={{ duration: 200 }}>
        <VirtualMasonry
          total={artworks.total}
          aspects={artworks.aspects()}
          minColumnWidth={narrow ? 150 : 200}
          initialIndex={anchorIndex}
          onrange={(start, end) => artworks.ensure(start, end)}
        >
          {#snippet children(index)}
            {@render artworkCell(index)}
          {/snippet}
        </VirtualMasonry>
      </div>
    {:else}
      <div class="h-full" in:swapIn={{ duration: 200 }}>
        <VirtualGrid
          total={artworks.total}
          minColumnWidth={narrow ? 150 : 200}
          initialIndex={anchorIndex}
          onrange={(start, end) => artworks.ensure(start, end)}
        >
          {#snippet children(index)}
            {@render artworkCell(index)}
          {/snippet}
        </VirtualGrid>
      </div>
    {/if}
  </main>
</div>

{#if picked.size > 0}
  <!-- 批量操作条；bottom 加 safe-area，别被全面屏手势条压住 -->
  <div
    class="pointer-events-none fixed bottom-[calc(1.25rem+env(safe-area-inset-bottom))] left-1/2 z-40 -translate-x-1/2"
    transition:fly={{ y: 64, duration: 240, easing: cubicOut }}
  >
    <div
      class="pointer-events-auto flex flex-wrap items-center gap-3 rounded-card border
             border-surface-700 bg-surface-900/95 px-4 py-2.5 shadow-2xl backdrop-blur"
    >
      <span class="text-xs text-soft">
        已选 <span class="font-mono font-semibold text-accent-400">{picked.size}</span> 项
      </span>

      <button
        type="button"
        class="cursor-pointer rounded-md px-3 py-1.5 text-xs font-medium transition disabled:opacity-60
               {confirmingBatch ? 'bg-rose-600 text-white hover:bg-rose-500' : 'bg-accent-600 text-on-accent hover:bg-accent-500'}"
        disabled={removing}
        onclick={() => {
          if (confirmingBatch) void remove([...picked]);
          else confirmingBatch = true;
        }}
      >
        {#if removing}
          移除中…
        {:else if confirmingBatch}
          确认移除 {picked.size} 项？
        {:else}
          移除选中
        {/if}
      </button>

      <button
        type="button"
        class="cursor-pointer rounded-md border border-surface-700 px-3 py-1.5 text-xs text-soft transition hover:border-surface-600 hover:text-fg"
        onclick={() => {
          picked = new Set();
          confirmingBatch = false;
        }}
      >
        取消选择
      </button>

      <span class="text-[10px] text-mark">只从作品库移除，不删除原图；重扫也不会再收回来</span>
    </div>
  </div>
{/if}

{#if selected}
  <ArtworkLightbox
    artwork={selected}
    onclose={() => {
      selected = null;
      selectedIndex = null;
    }}
    onnavigate={(delta) => void navigate(delta)}
    onfilterartist={(uid) => {
      // 只看这一位画师：收掉弹窗，把筛选条上的画师也同步过去
      selected = null;
      selectedIndex = null;
      artworks.artistUids = [uid];
      void applyFilter();
    }}
    onfiltertag={(tag) => {
      // 标签同理：收掉弹窗并让筛选条上的胶囊也亮起来
      selected = null;
      selectedIndex = null;
      artworks.pixivTags = [tag];
      void applyFilter();
    }}
  />
{/if}

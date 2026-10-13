<script lang="ts">
  import { onMount } from "svelte";
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";

  import ArtworksView from "$lib/components/ArtworksView.svelte";
  import ImageGrid from "$lib/components/ImageGrid.svelte";
  import Lightbox from "$lib/components/Lightbox.svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import Toolbar from "$lib/components/Toolbar.svelte";
  import { api } from "$lib/api";
  import { artworks } from "$lib/artworks.svelte";
  import { handleBack, registerBackCloser } from "$lib/backStack.svelte";
  /**
   * 视图切换的淡入。
   *
   * **刻意只有 opacity、没有位移**：位移需要 `transform`，而带 `transform` 的元素
   * 会成为其后代里 `position: fixed` 的包含块 —— 筛选浮层、批量操作条、检视弹窗
   * 就都不再相对视口定位，会整块跳到屏幕中间。`opacity` 只创建栈上下文，不创建
   * 包含块，是这里唯一安全的选择。
   */
  function fadeOnly(_node: HTMLElement, params: { duration?: number } = {}) {
    return {
      duration: params.duration ?? 180,
      easing: cubicOut,
      css: (t: number) => `opacity: ${t};`,
    };
  }

  import { gallery } from "$lib/gallery.svelte";
  import type { Image } from "$lib/types";

  let opened = $state<Image | null>(null);
  /** 手机上侧栏抽屉是否拉出（md 以上侧栏常驻，此值无效果） */
  let navOpen = $state(false);
  let noticeTimer: ReturnType<typeof setTimeout> | undefined;

  function onAndroidBack(): void {
    // 关闭栈里还有浮层（弹窗/抽屉/…）就只关最上层，否则真的退出应用
    if (handleBack()) return;
    void api.exitApp();
  }

  onMount(() => {
    void gallery.init();
    // MainActivity 把全面屏返回手势转发成这个事件（桌面上没有，无副作用）
    window.addEventListener("lc-android-back", onAndroidBack);
    return () => {
      void gallery.dispose();
      window.removeEventListener("lc-android-back", onAndroidBack);
    };
  });

  // 换筛选维度后自动收起抽屉（手机上点完侧栏就该回到内容）
  $effect(() => {
    void gallery.selection;
    navOpen = false;
  });

  // 抽屉开着时接住系统返回：按返回先收抽屉，而不是退出应用
  $effect(() => {
    if (!navOpen) return;
    return registerBackCloser(() => (navOpen = false));
  });

  // 作品库 / 标签图库都是侧栏里的原地切换视图，但对用户来说它们就是「另一个
  // 页面」：按返回应该退回全部图片，而不是一路退出到桌面（真机上撞到过）。
  // 弹窗在它上面打开时会后注册，所以先关弹窗、再退视图，顺序是对的。
  $effect(() => {
    const kind = gallery.selection.kind;
    if (kind !== "artworks" && kind !== "tagGallery") return;
    return registerBackCloser(() => gallery.select({ kind: "all" }));
  });

  /**
   * 侧栏「作品库 / 标签图库」的切换 -> 把内容区的筛选条件同步过去。
   *
   * 两条入口必须**各是各的**：
   * - 点标签图库：套上它的条件（条件背在 `selection` 里 —— 内置那两条压根不在
   *   库里，所以直接读，不用查表）
   * - 点作品库 (PID)：**清掉筛选**，回到「不筛的作品库」
   *
   * 后半条是真机撞出来的：早先这里只重置了书签、没清筛选，于是点过「R-18」
   * 再点「作品库 (PID)」，作品库继续显示 R-18 的内容 —— 两个入口串在一起，
   * 看上去就像是点错了页面。
   *
   * `key` 记下来是为了「同一个入口被重复点两次时不重复拉取」—— `setFilter`
   * 会作废所有块并重拉，白跑一趟肉眼可见。
   */
  let appliedEntryKey: string | null = null;
  $effect(() => {
    const selection = gallery.selection;
    // 其余几种选中项（全部图片 / 收藏 / 图库 / 相册 / 标签）显示的不是作品库
    // 视图，内容区根本不在场，不用管
    if (selection.kind !== "artworks" && selection.kind !== "tagGallery") {
      appliedEntryKey = null;
      return;
    }

    const key = selection.kind === "artworks" ? "entry:artworks" : `entry:gallery:${selection.key}`;
    if (key === appliedEntryKey) return;
    appliedEntryKey = key;

    if (selection.kind === "artworks") {
      // 已经是干净的就不必再拉一遍（首次进作品库走的就是这条路）
      if (artworks.filtered) void artworks.setFilter({});
      return;
    }

    void artworks.setFilter({
      artistUids: [],
      pixivTags: selection.tags,
      rating: selection.rating,
    });
  });

  // 操作结果提示 3 秒后自动消失
  $effect(() => {
    if (!gallery.notice) return;
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => (gallery.notice = null), 3000);
    return () => clearTimeout(noticeTimer);
  });

  async function navigate(delta: number): Promise<void> {
    const current = opened;
    if (!current) return;
    const index = gallery.images.findIndex((item) => item.id === current.id);
    if (index < 0) return;

    const next = index + delta;
    // 撞到已加载的边界就再要一页，否则一直按 → 会停在第一批 120 张上
    if (next >= gallery.images.length && gallery.hasMore) await gallery.loadMore();

    const target = gallery.images[next];
    if (target) opened = target;
  }
</script>

<div class="flex h-full w-full overflow-hidden bg-surface-950">
  <!-- 手机抽屉的背幕 -->
  {#if navOpen}
    <div
      class="fixed inset-0 z-30 bg-black/50 md:hidden"
      aria-hidden="true"
      onclick={() => (navOpen = false)}
    ></div>
  {/if}

  <Sidebar open={navOpen} onclose={() => (navOpen = false)} />

  <div class="flex min-w-0 flex-1 flex-col">
    <!-- 作品库是侧栏里的一个原地切换视图（与全部图片 / 收藏同一套交互），
         不再是独立页面 —— 内容区整个交给 ArtworksView，含它自己的工具条。

         切换带一层淡入：以前是「啪一下换掉」，在侧栏点两下会觉得整个界面在闪。

         **只淡不位移**：位移要写 `transform`，而 `transform` 会让这个容器变成
         固定定位元素的包含块 —— 里面那些 `fixed` 的浮层（筛选面板、批量操作条、
         检视弹窗）会突然改为相对它定位，跑到屏幕中间去。`opacity` 只建栈上下文，
         不建包含块，安全。 -->
    {#if gallery.selection.kind === "artworks" || gallery.selection.kind === "tagGallery"}
      <div class="flex min-h-0 flex-1 flex-col" in:fadeOnly={{ duration: 180 }}>
        <ArtworksView onnav={() => (navOpen = true)} />
      </div>
    {:else}
      <div class="flex min-h-0 flex-1 flex-col" in:fadeOnly={{ duration: 180 }}>
        <Toolbar onnav={() => (navOpen = true)} />
        <!-- 滚动交给 ImageGrid 里的 VirtualGrid 自己做（它要有固定的视口高度才能
             算可视区间）；这里再套一层 overflow-y-auto 会变成嵌套滚动 -->
        <main class="min-h-0 flex-1">
          <ImageGrid onopen={(image) => (opened = image)} />
        </main>
      </div>
    {/if}
  </div>
</div>

{#if opened}
  <Lightbox
    image={opened}
    onclose={() => (opened = null)}
    onnavigate={navigate}
    onfilterartist={(uid, name) => {
      // 先关弹窗再切筛选：弹窗打开时列表在它下面，切了也看不见
      opened = null;
      gallery.select({ kind: "artist", uid, name });
    }}
    onfiltertag={(tag) => {
      opened = null;
      gallery.select({ kind: "pixivTag", tag });
    }}
  />
{/if}

{#if gallery.error || gallery.notice}
  <!-- bottom 加 safe-area：全面屏手势条别压住提示 -->
  <div
    class="pointer-events-none fixed bottom-[calc(1rem+env(safe-area-inset-bottom))] left-1/2 z-60 -translate-x-1/2"
    transition:fly={{ y: 40, duration: 220, easing: cubicOut }}
  >
    {#if gallery.error}
      <div
        class="pointer-events-auto flex max-w-2xl items-start gap-3 rounded-lg border border-rose-900/60
               bg-rose-950/90 px-4 py-2.5 text-sm text-rose-200 shadow-xl backdrop-blur"
      >
        <span class="min-w-0 break-words">{gallery.error}</span>
        <button
          type="button"
          class="cursor-pointer text-rose-400 transition hover:text-rose-200"
          aria-label="关闭提示"
          onclick={() => (gallery.error = null)}
        >
          ×
        </button>
      </div>
    {:else if gallery.notice}
      <div
        class="pointer-events-auto rounded-lg border border-surface-700 bg-surface-850/95 px-4 py-2.5
               text-sm text-fg shadow-xl backdrop-blur"
      >
        {gallery.notice}
      </div>
    {/if}
  </div>
{/if}

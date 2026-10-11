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
  import { handleBack, registerBackCloser } from "$lib/backStack.svelte";
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
         不再是独立页面 —— 内容区整个交给 ArtworksView，含它自己的工具条 -->
    {#if gallery.selection.kind === "artworks"}
      <ArtworksView onnav={() => (navOpen = true)} />
    {:else}
      <Toolbar onnav={() => (navOpen = true)} />
      <main class="min-h-0 flex-1 overflow-y-auto">
        <ImageGrid onopen={(image) => (opened = image)} />
      </main>
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

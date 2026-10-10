<script lang="ts">
  /**
   * 阶段二：作品库（Pixiv 维度）。
   *
   * 列表用虚拟滚动渲染，缩略图由扫描阶段预生成、地址随列表一起返回，
   * 卡片直接就能画出来。原图只在点击后的检视弹窗里加载。
   */
  import { onMount } from "svelte";

  import { listen, type UnlistenFn } from "@tauri-apps/api/event";

  import { api, THUMBNAIL_PROGRESS_EVENT } from "$lib/api";
  import { artworks } from "$lib/artworks.svelte";
  import ArtworkCard from "$lib/components/ArtworkCard.svelte";
  import ArtworkLightbox from "$lib/components/ArtworkLightbox.svelte";
  import VirtualGrid from "$lib/components/VirtualGrid.svelte";
  import type { Artwork, ArtworkScanSummary, ThumbnailProgress } from "$lib/types";

  let scanning = $state(false);
  let summary = $state<ArtworkScanSummary | null>(null);
  let thumbProgress = $state<ThumbnailProgress | null>(null);
  let selected = $state<Artwork | null>(null);

  let progress = $derived(
    artworks.total === 0
      ? "尚未索引任何作品"
      : `共 ${artworks.total} 条作品页 · 已载入 ${artworks.ready}`,
  );

  onMount(() => {
    void artworks.bootstrap();

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
    };
  });

  /** 选择文件夹 -> 扫描入库 + 批量生成缩略图 -> 作废旧块重新拉取。 */
  async function pickAndScan(): Promise<void> {
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
    } catch (err) {
      artworks.error = String(err);
    } finally {
      scanning = false;
      thumbProgress = null;
    }
  }
</script>

<div class="flex h-screen flex-col bg-surface-950 text-slate-200">
  <header
    class="flex flex-wrap items-center gap-3 border-b border-surface-800 bg-surface-900 px-5 py-3"
  >
    <div class="min-w-0">
      <h1 class="text-sm font-semibold text-slate-100">作品库（PID 维度）</h1>
      <p class="text-[11px] tabular-nums text-slate-500">
        {#if artworks.busy && artworks.ready === 0}
          正在读取…
        {:else}
          {progress}
        {/if}
      </p>
    </div>

    <div class="flex-1"></div>

    <a
      href="/"
      class="rounded-md border border-surface-700 px-3 py-1.5 text-xs text-slate-300 transition hover:border-surface-600 hover:text-slate-100"
    >
      返回图库
    </a>

    <button
      type="button"
      class="flex cursor-pointer items-center gap-1.5 rounded-md bg-accent-600 px-3 py-1.5 text-sm
             font-medium text-white transition hover:bg-accent-500 disabled:cursor-not-allowed disabled:opacity-60"
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
        选择文件夹并扫描
      {/if}
    </button>
  </header>

  {#if summary}
    <div class="border-b border-surface-800 bg-surface-850 px-5 py-2 text-[11px] text-slate-400">
      上次扫描：发现 {summary.discovered} 个图片 · 命名匹配 {summary.matched} · 新入库
      <span class="text-accent-400">{summary.inserted}</span>
      · 去重跳过 <span class="text-amber-400">{summary.duplicates}</span>
      · 命名不符跳过 {summary.skipped}
      · 缩略图新生成 <span class="text-accent-400">{summary.thumbnailsGenerated}</span>
      / 复用 {summary.thumbnailsSkipped}
      {#if summary.thumbnailsFailed > 0}
        · 失败 <span class="text-rose-400">{summary.thumbnailsFailed}</span>
      {/if}
    </div>
  {/if}

  {#if artworks.error}
    <div class="m-5 rounded-md border border-rose-900/60 bg-rose-950/60 px-4 py-2.5 text-sm text-rose-200">
      {artworks.error}
    </div>
  {/if}

  <main class="min-h-0 flex-1">
    {#if !artworks.booted}
      <!-- 首块还没到，先铺一层骨架，避免闪一下「库是空的」 -->
      <div class="grid grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-3 p-4">
        {#each Array.from({ length: 12 }) as _, index (index)}
          <div class="aspect-square animate-pulse rounded-card bg-surface-850"></div>
        {/each}
      </div>
    {:else if artworks.total === 0}
      <div class="flex flex-col items-center justify-center gap-2 py-24 text-center">
        <p class="text-sm text-slate-400">作品库还是空的。</p>
        <p class="text-xs text-slate-500">
          点右上角「选择文件夹并扫描」，选一个含有 <code class="text-slate-400"
            >{'{pid}_p{num}.jpg'}</code
          > 文件的目录。
        </p>
      </div>
    {:else}
      <VirtualGrid
        total={artworks.total}
        minColumnWidth={200}
        onrange={(start, end) => artworks.ensure(start, end)}
      >
        {#snippet children(index)}
          {@const artwork = artworks.at(index)}
          {#if artwork}
            <ArtworkCard {artwork} onopen={(item) => (selected = item)} />
          {:else}
            <div class="h-full w-full animate-pulse rounded-card bg-surface-850"></div>
          {/if}
        {/snippet}
      </VirtualGrid>
    {/if}
  </main>
</div>

{#if selected}
  <!-- 按 id 作 key：换一张作品时把「已加载 / 加载失败」的状态一起重置 -->
  {#key selected.id}
    <ArtworkLightbox artwork={selected} onclose={() => (selected = null)} />
  {/key}
{/if}

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
  /** 弹窗当前这张在库里的序号，翻页要用。 */
  let selectedIndex = $state<number | null>(null);
  /** 忽略名单条数：被移出过、重扫也不会再收进来的作品。 */
  let ignoredCount = $state(0);
  /** 清空忽略名单的二次确认，与移除按钮同一套手势。 */
  let confirmingClear = $state(false);

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
      await refreshIgnored();
    } catch (err) {
      artworks.error = String(err);
    } finally {
      scanning = false;
      thumbProgress = null;
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
    } catch (err) {
      artworks.error = String(err);
    } finally {
      removing = false;
    }
  }
</script>

<div class="flex h-screen flex-col bg-surface-950 text-fg">
  <header
    class="flex flex-wrap items-center gap-3 border-b border-surface-800 bg-surface-900 px-5 py-3"
  >
    <div class="min-w-0">
      <h1 class="text-sm font-semibold text-fg">作品库（PID 维度）</h1>
      <p class="text-[11px] tabular-nums text-muted">
        {#if artworks.busy && artworks.ready === 0}
          正在读取…
        {:else}
          {progress}
        {/if}
      </p>
    </div>

    <div class="flex-1"></div>

    <a
      href="/appearance"
      class="rounded-md border border-surface-700 px-3 py-1.5 text-xs text-soft transition hover:border-surface-600 hover:text-fg"
    >
      外观
    </a>

    <a
      href="/"
      class="rounded-md border border-surface-700 px-3 py-1.5 text-xs text-soft transition hover:border-surface-600 hover:text-fg"
    >
      返回图库
    </a>

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
        选择文件夹并扫描
      {/if}
    </button>
  </header>

  {#if summary}
    <div class="border-b border-surface-800 bg-surface-850 px-5 py-2 text-[11px] text-muted">
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
    <div class="m-5 rounded-md border border-rose-900/60 bg-rose-950/60 px-4 py-2.5 text-sm text-rose-200">
      {artworks.error}
    </div>
  {/if}

  {#if artworks.notice}
    <div
      class="border-b border-surface-800 bg-accent-500/10 px-5 py-2 text-[11px] text-accent-400"
    >
      {artworks.notice}
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
        <p class="text-sm text-muted">作品库还是空的。</p>
        <p class="text-xs text-muted">
          点右上角「选择文件夹并扫描」，选一个含有 <code class="text-muted"
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
      </VirtualGrid>
    {/if}
  </main>
</div>

{#if picked.size > 0}
  <!-- 批量操作条 -->
  <div class="pointer-events-none fixed bottom-5 left-1/2 z-40 -translate-x-1/2">
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
  />
{/if}

<script lang="ts">
  /**
   * 阶段一：作品（Pixiv）维度页面。
   *
   * 按需求，本页**只展示文件路径与 PID 文本，不加载任何原图**
   * —— 缩略图留到下一阶段处理。
   */
  import { onMount } from "svelte";

  import { api } from "$lib/api";
  import type { Artwork, ArtworkScanSummary } from "$lib/types";

  const PAGE_SIZE = 60;

  let items = $state<Artwork[]>([]);
  let total = $state(0);
  let offset = $state(0);
  let loading = $state(false);
  let error = $state<string | null>(null);

  let scanning = $state(false);
  let summary = $state<ArtworkScanSummary | null>(null);

  let pageCount = $derived(Math.max(1, Math.ceil(total / PAGE_SIZE)));
  let pageIndex = $derived(Math.floor(offset / PAGE_SIZE) + 1);
  let rangeStart = $derived(total === 0 ? 0 : offset + 1);
  let rangeEnd = $derived(Math.min(offset + PAGE_SIZE, total));

  onMount(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    error = null;
    try {
      const page = await api.listArtworks(offset, PAGE_SIZE);
      items = page.items;
      total = page.total;
    } catch (err) {
      error = String(err);
    } finally {
      loading = false;
    }
  }

  /** 选择文件夹 -> 调 Rust 端扫描命令 -> 回到第一页刷新列表。 */
  async function pickAndScan(): Promise<void> {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({
      directory: true,
      multiple: false,
      title: "选择包含 {pid}_p{num}.jpg 的目录",
    });
    if (typeof picked !== "string") return;

    scanning = true;
    error = null;
    try {
      summary = await api.scanArtworksFolder(picked);
      offset = 0;
      await load();
    } catch (err) {
      error = String(err);
    } finally {
      scanning = false;
    }
  }

  function goTo(next: number): void {
    const clamped = Math.max(0, Math.min(next, (pageCount - 1) * PAGE_SIZE));
    if (clamped === offset) return;
    offset = clamped;
    void load();
  }
</script>

<div class="min-h-screen bg-surface-950 text-slate-200">
  <header class="flex flex-wrap items-center gap-3 border-b border-surface-800 bg-surface-900 px-5 py-3">
    <div class="min-w-0">
      <h1 class="text-sm font-semibold text-slate-100">作品库（PID 维度）</h1>
      <p class="text-[11px] text-slate-500">
        {#if loading}
          正在读取…
        {:else}
          共 {total} 条作品页
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
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4 animate-spin">
          <path stroke-linecap="round" d="M12 3a9 9 0 019 9" />
        </svg>
        扫描中…
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
    </div>
  {/if}

  {#if error}
    <div class="m-5 rounded-md border border-rose-900/60 bg-rose-950/60 px-4 py-2.5 text-sm text-rose-200">
      {error}
    </div>
  {/if}

  <main class="p-5">
    {#if items.length === 0 && !loading}
      <div class="flex flex-col items-center justify-center gap-2 py-24 text-center">
        <p class="text-sm text-slate-400">作品库还是空的。</p>
        <p class="text-xs text-slate-500">
          点右上角「选择文件夹并扫描」，选一个含有 <code class="text-slate-400">{'{pid}_p{num}.jpg'}</code> 文件的目录。
        </p>
      </div>
    {:else}
      <!-- 基础的 CSS 网格：每格只显示路径文本与 PID，不加载图片 -->
      <div class="grid grid-cols-[repeat(auto-fill,minmax(220px,1fr))] gap-3">
        {#each items as item (item.id)}
          <article
            class="flex flex-col gap-2 rounded-card border border-surface-800 bg-surface-900 p-3
                   transition hover:border-surface-700"
          >
            <div class="flex items-center justify-between gap-2">
              <span
                class="rounded bg-accent-500/15 px-2 py-0.5 font-mono text-xs font-semibold text-accent-400"
                title="Pixiv 作品号"
              >
                {item.pid}
              </span>
              <span class="shrink-0 text-[10px] text-slate-500">p{item.page}</span>
            </div>

            <p
              class="font-mono text-[11px] leading-relaxed break-all text-slate-400"
              title={item.filePath}
            >
              {item.filePath}
            </p>
          </article>
        {/each}
      </div>

      <!-- 分页 -->
      <nav class="mt-6 flex items-center justify-center gap-3 text-xs">
        <button
          type="button"
          class="cursor-pointer rounded-md border border-surface-700 px-3 py-1.5 text-slate-300
                 transition hover:border-surface-600 hover:text-slate-100
                 disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:border-surface-700"
          disabled={offset === 0 || loading}
          onclick={() => goTo(offset - PAGE_SIZE)}
        >
          上一页
        </button>

        <span class="tabular-nums text-slate-500">
          {#if total === 0}
            0 / 0
          {:else}
            {rangeStart}–{rangeEnd} · 第 {pageIndex} / {pageCount} 页
          {/if}
        </span>

        <button
          type="button"
          class="cursor-pointer rounded-md border border-surface-700 px-3 py-1.5 text-slate-300
                 transition hover:border-surface-600 hover:text-slate-100
                 disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:border-surface-700"
          disabled={offset + PAGE_SIZE >= total || loading}
          onclick={() => goTo(offset + PAGE_SIZE)}
        >
          下一页
        </button>
      </nav>
    {/if}
  </main>
</div>

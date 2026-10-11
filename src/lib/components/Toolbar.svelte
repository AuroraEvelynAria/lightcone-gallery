<script lang="ts">
  import { gallery } from "$lib/gallery.svelte";
  import type { SortBy } from "$lib/types";

  /** 画幅选项。标签只用一个字，塞在工具条里不占地。 */
  const ORIENTATIONS = [
    { value: "landscape", label: "横", title: "只看横屏（宽 > 高）" },
    { value: "portrait", label: "竖", title: "只看竖屏（宽 < 高）" },
    { value: "square", label: "方", title: "只看方形（宽 = 高）" },
  ] as const;

  const SORT_OPTIONS: { value: SortBy; label: string }[] = [
    { value: "modifiedAt", label: "修改时间" },
    { value: "indexedAt", label: "加入时间" },
    { value: "filename", label: "文件名" },
    { value: "fileSize", label: "文件大小" },
    { value: "rating", label: "评分" },
  ];

  let {
    onnav,
  }: {
    /** 手机上的汉堡按钮回调（拉出侧栏抽屉）；桌面不渲染 */
    onnav?: () => void;
  } = $props();

  let searchInput = $state(gallery.search);
  let debounce: ReturnType<typeof setTimeout> | undefined;

  function onSearchInput(): void {
    clearTimeout(debounce);
    debounce = setTimeout(() => {
      gallery.search = searchInput;
      void gallery.refreshImages();
    }, 250);
  }

  function onSortChange(event: Event): void {
    gallery.sortBy = (event.currentTarget as HTMLSelectElement).value as SortBy;
    void gallery.refreshImages();
  }

  function onRatingChange(event: Event): void {
    gallery.minimumRating = Number((event.currentTarget as HTMLSelectElement).value);
    void gallery.refreshImages();
  }
</script>

<!-- 手机上工具条允许换行（min-h-14），桌面保持单行 h-14 -->
<header
  class="flex min-h-14 shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-b border-surface-800
         bg-surface-900 px-4 md:h-14 md:flex-nowrap md:gap-3"
>
  <!-- 汉堡：手机上拉出侧栏抽屉 -->
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
    <h1 class="truncate text-sm font-semibold text-fg">{gallery.selectionLabel}</h1>
    <p class="truncate text-[11px] text-muted">
      {#if gallery.loading}
        正在读取…
      {:else}
        共 {gallery.total} 张
      {/if}
    </p>
  </div>

  <div class="hidden flex-1 md:block"></div>

  <label class="relative w-full min-w-0 md:w-auto">
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      class="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted"
    >
      <path stroke-linecap="round" stroke-linejoin="round" d="M21 21l-5.197-5.197m0 0A7.5 7.5 0 105.196 5.196a7.5 7.5 0 0010.607 10.607z" />
    </svg>
    <input
      type="search"
      placeholder="搜索文件名或路径…"
      class="w-full rounded-md border border-surface-700 bg-surface-850 py-1.5 pr-3 pl-8 text-sm
             text-fg placeholder:text-muted outline-none focus:border-accent-500 md:w-56"
      bind:value={searchInput}
      oninput={onSearchInput}
    />
  </label>

  <select
    class="cursor-pointer rounded-md border border-surface-700 bg-surface-850 px-2 py-1.5 text-sm text-soft outline-none focus:border-accent-500"
    value={gallery.minimumRating}
    onchange={onRatingChange}
    title="按评分筛选"
  >
    <option value={0}>全部评分</option>
    <option value={3}>≥ 3 星</option>
    <option value={4}>≥ 4 星</option>
    <option value={5}>5 星</option>
  </select>

  <!--
    画幅筛选。做成三个小按钮而不是下拉：选项只有三种，平铺出来一眼就能选，
    不必先展开再选（原生下拉在自动化里也驱动不了）。
    都不选 = 不限，再点一次选中的那个就取消。
  -->
  <div class="flex items-center gap-0.5 rounded-md border border-surface-700 bg-surface-850 p-0.5">
    {#each ORIENTATIONS as option (option.value)}
      <button
        type="button"
        class="cursor-pointer rounded px-2 py-1 text-xs transition {gallery.orientation ===
        option.value
          ? 'bg-accent-500/20 text-accent-400'
          : 'text-soft hover:text-fg'}"
        aria-pressed={gallery.orientation === option.value}
        title={option.title}
        onclick={() => {
          gallery.orientation =
            gallery.orientation === option.value ? null : option.value;
          void gallery.refreshImages();
        }}
      >
        {option.label}
      </button>
    {/each}
  </div>

  <select
    class="cursor-pointer rounded-md border border-surface-700 bg-surface-850 px-2 py-1.5 text-sm text-soft outline-none focus:border-accent-500"
    value={gallery.sortBy}
    onchange={onSortChange}
    title="排序方式"
  >
    {#each SORT_OPTIONS as option (option.value)}
      <option value={option.value}>{option.label}</option>
    {/each}
  </select>

  <button
    type="button"
    class="cursor-pointer rounded-md border border-surface-700 bg-surface-850 p-1.5 text-muted transition hover:text-fg"
    title={gallery.sortDesc ? "当前：降序" : "当前：升序"}
    onclick={() => {
      gallery.sortDesc = !gallery.sortDesc;
      void gallery.refreshImages();
    }}
  >
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
      {#if gallery.sortDesc}
        <path stroke-linecap="round" stroke-linejoin="round" d="M19.5 13.5L12 21m0 0l-7.5-7.5M12 21V3" />
      {:else}
        <path stroke-linecap="round" stroke-linejoin="round" d="M4.5 10.5L12 3m0 0l7.5 7.5M12 3v18" />
      {/if}
    </svg>
  </button>

  <button
    type="button"
    class="flex cursor-pointer items-center gap-1.5 rounded-md bg-accent-600 px-3 py-1.5 text-sm
           font-medium text-on-accent transition hover:bg-accent-500 disabled:cursor-not-allowed disabled:opacity-60"
    disabled={gallery.scanning || gallery.libraries.length === 0}
    onclick={() => gallery.scan()}
  >
    {#if gallery.scanning}
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4 animate-spin">
        <path stroke-linecap="round" d="M12 3a9 9 0 019 9" />
      </svg>
      索引中…
    {:else}
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
        <path stroke-linecap="round" stroke-linejoin="round" d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0l3.181 3.183a8.25 8.25 0 0013.803-3.7M4.031 9.865a8.25 8.25 0 0113.803-3.7l3.181 3.182m0-4.991v4.99" />
      </svg>
      扫描全部
    {/if}
  </button>
</header>

{#if gallery.scanning && gallery.scanProgress}
  {@const progress = gallery.scanProgress}
  <div class="border-b border-surface-800 bg-surface-850 px-4 py-1.5 text-[11px] text-muted">
    {#if progress.phase === "discovering"}
      正在扫描「{progress.libraryName}」… 已发现 {progress.processed} 个文件
    {:else}
      正在索引「{progress.libraryName}」：
      {progress.processed} / {progress.total}
    {/if}
  </div>
{/if}

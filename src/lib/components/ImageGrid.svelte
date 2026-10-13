<script lang="ts">
  /**
   * 通用图库网格（虚拟滚动）。
   *
   * **为什么要虚拟化**：早先这里是 `{#each gallery.images}`，把**已加载的全渲染出来**，
   * 配一个底部哨兵做无限滚动。真机实测（CDP 在页面里量 rAF）是：
   * 数据已缓存时完美 120Hz（p99 8.4ms、零长任务），**一翻页就冒 50-60ms 的长任务、
   * 80+ 帧超过 16.7ms** —— 每页要一次性挂载 120 张卡片，而且 DOM 节点只增不减
   * （滑到底会到几万个）。换成 VirtualGrid 之后，无论加载了多少条，DOM 里始终
   * 只有视口那几十格。
   *
   * 只在「已加载的条数」上做无限滚动，**滚动条长度一开始就按 `total` 撑满** ——
   * 没加载到的格子显示骨架，而不是像旧版那样边滑边把页面撑长。
   */
  import { gallery } from "$lib/gallery.svelte";
  import type { Image } from "$lib/types";

  import ImageCard from "./ImageCard.svelte";
  import VirtualGrid from "./VirtualGrid.svelte";

  let { onopen }: { onopen: (image: Image) => void } = $props();

  // 与原来 `grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-3 p-4` 的取值对齐，
  // 换实现不改观感（ImageCard 本身就是 aspect-square，所以 aspect 用默认的 1）
  const MIN_COLUMN_WIDTH = 150;
  const GAP = 12;
  const PADDING = 16;
  /** 首屏骨架的格数：铺一屏多一点，够表达「在加载」就行 */
  const SKELETON_CELLS = 18;
</script>

{#if gallery.loading && gallery.images.length === 0}
  <div class="grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-3 p-4">
    {#each Array.from({ length: SKELETON_CELLS }) as _, index (index)}
      <div class="aspect-square animate-pulse rounded-card bg-surface-850"></div>
    {/each}
  </div>
{:else if gallery.total === 0}
  <div class="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.2" class="size-14 text-surface-600">
      <path
        stroke-linecap="round"
        stroke-linejoin="round"
        d="M2.25 15.75l5.159-5.159a2.25 2.25 0 013.182 0l5.159 5.159m-1.5-1.5l1.409-1.409a2.25 2.25 0 013.182 0l2.909 2.909M3.75 21h16.5A2.25 2.25 0 0022.5 18.75V5.25A2.25 2.25 0 0020.25 3H3.75A2.25 2.25 0 001.5 5.25v13.5A2.25 2.25 0 003.75 21z"
      />
    </svg>
    <p class="text-sm text-muted">
      {#if gallery.stats?.images === 0}
        图库还是空的。点左下角「添加图库」选择一个图片目录开始索引。
      {:else}
        「{gallery.selectionLabel}」下没有匹配的图片。
      {/if}
    </p>
  </div>
{:else}
  <div class="relative h-full min-h-0">
    <VirtualGrid
      total={gallery.total}
      minColumnWidth={MIN_COLUMN_WIDTH}
      gap={GAP}
      padding={PADDING}
      onrange={(_start, end) => void gallery.ensureLoaded(end)}
    >
      {#snippet children(index)}
        {@const image = gallery.images[index]}
        {#if image}
          <ImageCard {image} onselect={onopen} ontogglefavorite={(item) => gallery.toggleFavorite(item)} />
        {:else}
          <!-- 还没拉到的格子。网格总高已经按 total 撑开了，所以滑到未加载区域
               看到的是骨架，而不是「到底了」 -->
          <div class="h-full w-full animate-pulse rounded-card bg-surface-850"></div>
        {/if}
      {/snippet}
    </VirtualGrid>

    <!-- 续页提示：虚拟网格把内容区占满了，没有地方放页脚，所以做成悬浮胶囊 -->
    {#if gallery.loadingMore}
      <div
        class="pointer-events-none absolute bottom-4 left-1/2 -translate-x-1/2 rounded-full border
               border-surface-700 bg-surface-900/95 px-3 py-1 text-xs tabular-nums text-muted
               shadow-lg backdrop-blur"
      >
        正在加载… {gallery.images.length} / {gallery.total}
      </div>
    {/if}
  </div>
{/if}

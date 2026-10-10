<script lang="ts">
  import { gallery } from "$lib/gallery.svelte";
  import type { Image } from "$lib/types";

  import ImageCard from "./ImageCard.svelte";

  let { onopen }: { onopen: (image: Image) => void } = $props();

  // 列表底部哨兵：滚到底自动续页
  let sentinel = $state<HTMLElement | null>(null);

  $effect(() => {
    const node = sentinel;
    if (!node) return;

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) void gallery.loadMore();
      },
      { rootMargin: "600px" },
    );
    observer.observe(node);
    return () => observer.disconnect();
  });
</script>

{#if gallery.loading}
  <div class="grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-3 p-4">
    {#each Array.from({ length: 18 }) as _, index (index)}
      <div class="aspect-square animate-pulse rounded-card bg-surface-850"></div>
    {/each}
  </div>
{:else if gallery.images.length === 0}
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
  <div class="grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-3 p-4">
    {#each gallery.images as image (image.id)}
      <ImageCard {image} onselect={onopen} ontogglefavorite={(item) => gallery.toggleFavorite(item)} />
    {/each}
  </div>

  <div bind:this={sentinel} class="flex items-center justify-center py-6 text-xs text-muted">
    {#if gallery.loadingMore}
      正在加载…
    {:else if gallery.hasMore}
      已显示 {gallery.images.length} / {gallery.total}
    {:else}
      已到底部 · 共 {gallery.total} 张
    {/if}
  </div>
{/if}

<script lang="ts">
  /**
   * 通用图库的缩略图：进入视口附近才向后端要路径，避免一次性解码上千张图。
   *
   * 作品库（`/artworks`）不用这个组件 —— 那边的缩略图地址随列表一起返回，
   * 卡片直接渲染 `<img>` 即可，见 `ArtworkThumbnail.svelte`。
   *
   * 走 `withThumbnailSlot` 排队，防止滚动时同时抛出一整屏解码任务。
   */
  import { convertFileSrc } from "@tauri-apps/api/core";

  import { api } from "$lib/api";
  import { withThumbnailSlot } from "$lib/thumbnailQueue";

  let { id, size = 320, alt = "" }: { id: number; size?: number; alt?: string } = $props();

  let element = $state<HTMLElement | null>(null);
  let source = $state<string | null>(null);
  let failed = $state(false);

  // 非响应式标记：记录当前正在请求 / 已经取到的是哪一条
  let requestedId: number | null = null;

  /**
   * 最近的滚动祖先，用作观察器的 root。
   *
   * 不能省略 root：默认视口时，滚动容器自身的裁剪会把 rootMargin 完全吃掉，
   * 于是「提前一屏预取」永远不会发生，快速滚动时满屏骨架。
   */
  function scrollContainer(node: HTMLElement): Element | null {
    let parent = node.parentElement;
    while (parent) {
      const { overflowY } = getComputedStyle(parent);
      if (overflowY === "auto" || overflowY === "scroll") return parent;
      parent = parent.parentElement;
    }
    return null;
  }

  $effect(() => {
    const node = element;
    const currentId = id;
    const currentSize = size;
    if (!node) return;

    // 依赖 currentId：列表刷新后同一个实例可能被复用去渲染另一张图，
    // id 一变就得把上一张的状态清干净，否则会显示出错位的缩略图。
    source = null;
    failed = false;
    requestedId = null;

    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((entry) => entry.isIntersecting)) return;
        observer.disconnect();
        void load(currentId, currentSize);
      },
      // 提前一屏预取，滚动时不会看到大片占位
      { root: scrollContainer(node), rootMargin: "400px" },
    );

    observer.observe(node);
    return () => observer.disconnect();
  });

  async function load(targetId: number, targetSize: number): Promise<void> {
    if (requestedId === targetId) return;
    requestedId = targetId;
    try {
      const path = await withThumbnailSlot(() => api.thumbnailFor(targetId, targetSize));
      // 请求排队期间 id 可能又被换掉（快速滚动），过期的结果直接丢掉
      if (requestedId !== targetId) return;
      source = convertFileSrc(path);
    } catch {
      if (requestedId === targetId) failed = true;
    }
  }
</script>

<div bind:this={element} class="h-full w-full">
  {#if source}
    <img
      src={source}
      {alt}
      loading="lazy"
      decoding="async"
      draggable="false"
      class="h-full w-full object-cover"
      onerror={() => (failed = true)}
    />
  {:else if failed}
    <div class="flex h-full w-full items-center justify-center bg-surface-850 text-mark">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" class="size-8">
        <path
          stroke-linecap="round"
          stroke-linejoin="round"
          d="M2.25 15.75l5.159-5.159a2.25 2.25 0 013.182 0l5.159 5.159m-1.5-1.5l1.409-1.409a2.25 2.25 0 013.182 0l2.909 2.909M3.75 21h16.5A2.25 2.25 0 0022.5 18.75V5.25A2.25 2.25 0 0020.25 3H3.75A2.25 2.25 0 001.5 5.25v13.5A2.25 2.25 0 003.75 21z"
        />
      </svg>
    </div>
  {:else}
    <div class="h-full w-full animate-pulse bg-surface-850"></div>
  {/if}
</div>

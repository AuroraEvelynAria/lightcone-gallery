<script lang="ts">
  /**
   * 作品缩略图。
   *
   * 正常路径是「直接用列表里带回来的 `thumbnailPath`」—— 零 IPC，`<img>` 自己就加载了。
   * 只有缓存被清空 / 盘上文件被删导致加载失败时才向后端要一次重生成；
   * 那条路要走并发闸门，否则整屏卡片会同时抛请求。
   *
   * 状态全部按 `artwork.id` 记账而不是靠 effect 重置：虚拟滚动会复用组件实例，
   * id 一变，上一张的「已恢复 / 已失败」自然就不匹配了。
   */
  import { convertFileSrc } from "@tauri-apps/api/core";

  import { api } from "$lib/api";
  import { withThumbnailSlot } from "$lib/thumbnailQueue";
  import type { Artwork } from "$lib/types";

  let { artwork }: { artwork: Artwork } = $props();

  /** 回退成功后拿到的地址，连同它属于哪一条一起记。 */
  let recovered = $state<{ id: number; src: string } | null>(null);
  /** 回退失败的那一条 id。 */
  let failedId = $state<number | null>(null);

  /**
   * 已经画出像素的那一条 id。
   *
   * 按 id 记账而不是靠 effect 重置：虚拟滚动会复用组件实例，id 一变上一张的
   * 「已画好」自然就不匹配了 —— 与 `recovered` / `failedId` 同一套理由。
   *
   * 用 CSS 过渡而不是 Svelte `transition:`：虚拟滚动下每滚一屏都会新建十几个
   * `<img>`，Svelte 过渡每次都要跑一遍 JS 调度；而这里只需一次 opacity 补间，
   * 交给合成器最省。淡入也只动 opacity —— 滚动时不会因为动画掉帧。
   */
  let paintedId = $state<number | null>(null);

  let source = $derived(
    recovered?.id === artwork.id
      ? recovered.src
      : artwork.thumbnailPath
        ? convertFileSrc(artwork.thumbnailPath)
        : null,
  );
  let isFailed = $derived(failedId === artwork.id);

  // 非响应式标记：防止坏图反复触发请求
  let recovering = false;

  /** 向后端要一次重生成。失败就停在占位状态，不再重试。 */
  async function recover(): Promise<void> {
    const target = artwork.id;
    if (recovering || failedId === target) return;
    recovering = true;
    try {
      const path = await withThumbnailSlot(() => api.thumbnailForArtwork(target));
      // 排队期间实例可能已被复用去渲染别的条目，过期的结果直接丢掉
      if (artwork.id !== target) return;
      artwork.thumbnailPath = path;
      recovered = { id: target, src: convertFileSrc(path) };
    } catch {
      if (artwork.id === target) failedId = target;
    } finally {
      recovering = false;
    }
  }

  // 扫描阶段生成失败的话路径列会是 null，这里补一次
  $effect(() => {
    if (artwork.thumbnailPath === null) void recover();
  });
</script>

<div class="h-full w-full">
  {#if source && !isFailed}
    <img
      src={source}
      alt={artwork.pid}
      loading="lazy"
      decoding="async"
      draggable="false"
      class="h-full w-full object-cover transition-opacity duration-150 {paintedId === artwork.id
        ? 'opacity-100'
        : 'opacity-0'}"
      onload={() => (paintedId = artwork.id)}
      onerror={() => void recover()}
    />
  {:else if isFailed}
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

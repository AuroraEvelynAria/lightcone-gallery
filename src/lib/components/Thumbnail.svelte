<script lang="ts">
  /**
   * 缩略图：进入视口附近才请求后端生成 / 读取，避免一次性解码上千张图。
   */
  import { convertFileSrc } from "@tauri-apps/api/core";

  import { api } from "$lib/api";

  let {
    imageId,
    size = 320,
    alt = "",
  }: { imageId: number; size?: number; alt?: string } = $props();

  let element = $state<HTMLElement | null>(null);
  let source = $state<string | null>(null);
  let failed = $state(false);

  // 非响应式标记：观察器只应挂载一次
  let requested = false;

  $effect(() => {
    const node = element;
    if (!node || requested) return;

    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((entry) => entry.isIntersecting)) return;
        observer.disconnect();
        void load();
      },
      // 提前一屏预取，滚动时不会看到大片占位
      { rootMargin: "400px" },
    );

    observer.observe(node);
    return () => observer.disconnect();
  });

  async function load(): Promise<void> {
    if (requested) return;
    requested = true;
    try {
      source = convertFileSrc(await api.thumbnailFor(imageId, size));
    } catch {
      failed = true;
    }
  }
</script>

<div bind:this={element} class="h-full w-full">
  {#if source}
    <img
      src={source}
      {alt}
      loading="lazy"
      draggable="false"
      class="h-full w-full object-cover"
      onerror={() => (failed = true)}
    />
  {:else if failed}
    <div class="flex h-full w-full items-center justify-center bg-surface-850 text-slate-600">
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

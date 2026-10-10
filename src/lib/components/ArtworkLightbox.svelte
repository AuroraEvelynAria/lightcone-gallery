<script lang="ts">
  /**
   * 作品检视弹窗：显示**原图**。
   *
   * 整个项目的硬约束是「列表只拉缩略图，原图只在弹窗里出现」，所以这里是原图
   * 唯一会被加载的地方。当前刻意做成最朴素的实现 —— RhinE 工业科技风的
   * 3D 视差弹窗是阶段四的事，这一版只负责把原图正确地显示出来。
   */
  import { convertFileSrc } from "@tauri-apps/api/core";

  import type { Artwork } from "$lib/types";

  let { artwork, onclose }: { artwork: Artwork; onclose: () => void } = $props();

  let loaded = $state(false);
  let failed = $state(false);

  let source = $derived(convertFileSrc(artwork.originalPath));

  $effect(() => {
    const onkeydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onclose();
    };
    window.addEventListener("keydown", onkeydown);
    return () => window.removeEventListener("keydown", onkeydown);
  });
</script>

<div
  class="fixed inset-0 z-50 flex flex-col bg-black/90 backdrop-blur-sm"
  role="dialog"
  aria-modal="true"
  aria-label={`作品 ${artwork.pid} 第 ${artwork.page} 页`}
>
  <!-- 背景层：点空白处关闭 -->
  <button
    type="button"
    class="absolute inset-0 cursor-zoom-out"
    aria-label="关闭"
    onclick={onclose}
  ></button>

  <header class="relative z-10 flex items-center gap-3 px-5 py-3 text-fg">
    <span class="font-mono text-sm font-semibold text-accent-400" title="Pixiv 作品号">
      {artwork.pid}
    </span>
    <span class="text-xs text-muted">p{artwork.page}</span>
    <div class="flex-1"></div>
    <button
      type="button"
      class="cursor-pointer rounded-md border border-surface-700 px-3 py-1.5 text-xs transition
             hover:border-surface-600 hover:text-white"
      onclick={onclose}
    >
      关闭 (Esc)
    </button>
  </header>

  <div class="relative z-10 flex min-h-0 flex-1 items-center justify-center p-4">
    {#if failed}
      <p class="max-w-xl text-center text-sm text-rose-300">
        原图加载失败，文件可能已被移动或删除：<br />
        <span class="font-mono text-xs break-all text-rose-200/70">{artwork.originalPath}</span>
      </p>
    {:else}
      <img
        src={source}
        alt={`${artwork.pid} p${artwork.page}`}
        draggable="false"
        class="max-h-full max-w-full rounded-card object-contain shadow-2xl transition-opacity
               {loaded ? 'opacity-100' : 'opacity-0'}"
        onload={() => (loaded = true)}
        onerror={() => (failed = true)}
      />
      {#if !loaded}
        <div
          class="absolute size-10 animate-spin rounded-full border-2 border-surface-600 border-t-accent-400"
        ></div>
      {/if}
    {/if}
  </div>

  <footer class="relative z-10 px-5 pb-4 text-center font-mono text-[11px] break-all text-muted">
    {artwork.originalPath}
  </footer>
</div>

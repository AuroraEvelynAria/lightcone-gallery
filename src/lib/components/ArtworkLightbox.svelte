<script lang="ts">
  /**
   * 作品检视弹窗：光锥 3D 视差 + 鼠标跟随扫光。
   *
   * 视差本体、缩放平移、全屏都在 `ImageViewer` 里 —— 通用图库那个弹窗用的是
   * 同一个内核，两边手感必须一致，所以这里只剩作品自己的一条信息栏。
   */
  import { convertFileSrc } from "@tauri-apps/api/core";

  import type { Artwork } from "$lib/types";

  import ImageViewer from "./ImageViewer.svelte";

  let {
    artwork,
    onclose,
    onnavigate,
  }: {
    artwork: Artwork;
    onclose: () => void;
    /** 上一张 / 下一张。作品库里翻页要等对应数据块到位，所以是异步的 */
    onnavigate?: (delta: number) => void;
  } = $props();
</script>

<div
  class="fixed inset-0 z-50 flex flex-col bg-black/88 backdrop-blur-sm"
  role="dialog"
  aria-modal="true"
  aria-label={`作品 ${artwork.pid} 第 ${artwork.page} 页`}
>
  <header class="relative z-10 flex items-center gap-3 px-5 py-3 text-soft">
    <span class="font-mono text-sm font-semibold text-accent-400" title="Pixiv 作品号">
      {artwork.pid}
    </span>
    <span class="text-xs text-muted">p{artwork.page}</span>
    <div class="flex-1"></div>
    <button
      type="button"
      class="cursor-pointer rounded-md border border-surface-700 px-3 py-1.5 text-xs transition
             hover:border-surface-600 hover:text-fg"
      onclick={onclose}
    >
      关闭 (Esc)
    </button>
  </header>

  <div class="relative min-h-0 flex-1">
    <ImageViewer
      src={convertFileSrc(artwork.originalPath)}
      alt={`${artwork.pid} p${artwork.page}`}
      failurePath={artwork.originalPath}
      {onclose}
      {onnavigate}
    >
      {#snippet meta()}
        <span class="viewer-chip">p{artwork.page}</span>
        <span class="viewer-caption" data-selectable>{artwork.originalPath}</span>
      {/snippet}
    </ImageViewer>
  </div>
</div>

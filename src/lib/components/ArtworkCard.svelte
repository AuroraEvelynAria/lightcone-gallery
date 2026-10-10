<script lang="ts">
  /**
   * 作品格子：缩略图 + PID 角标，点击打开检视弹窗（弹窗里才加载原图）。
   *
   * 缩略图地址来自列表自带的 `thumbnailPath`，卡片不额外发 IPC。
   */
  import type { Artwork } from "$lib/types";

  import ArtworkThumbnail from "./ArtworkThumbnail.svelte";

  let {
    artwork,
    onopen,
  }: { artwork: Artwork; onopen: (artwork: Artwork) => void } = $props();
</script>

<button
  type="button"
  class="group relative h-full w-full cursor-pointer overflow-hidden rounded-card bg-surface-850
         ring-1 ring-white/5 transition hover:ring-2 hover:ring-accent-500/70
         focus:outline-none focus-visible:ring-2 focus-visible:ring-accent-400"
  title={artwork.originalPath}
  aria-label={`查看作品 ${artwork.pid} 第 ${artwork.page} 页`}
  onclick={() => onopen(artwork)}
>
  <div class="absolute inset-0">
    <ArtworkThumbnail {artwork} />
  </div>

  <!-- 信息条：PID 常显，原图路径悬停时浮出来 -->
  <div
    class="pointer-events-none absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/85 to-transparent
           px-2 pt-8 pb-1.5 text-left"
  >
    <div class="flex items-baseline justify-between gap-2">
      <span class="font-mono text-xs font-semibold text-accent-400" title="Pixiv 作品号">
        {artwork.pid}
      </span>
      <span class="shrink-0 text-[10px] tabular-nums text-soft">p{artwork.page}</span>
    </div>
    <p
      class="truncate font-mono text-[10px] text-muted opacity-0 transition group-hover:opacity-100"
    >
      {artwork.originalPath}
    </p>
  </div>
</button>

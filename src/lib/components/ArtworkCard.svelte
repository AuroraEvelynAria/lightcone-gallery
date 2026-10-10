<script lang="ts">
  /**
   * 作品格子：有损 WebP 缩略图 + PID 角标。
   *
   * 阶段二只做「缩略图 + 虚拟滚动」，点击后的视差检视弹窗按计划留到阶段三 / 阶段四。
   * 前端始终只拉缩略图，原图不出现在这里。
   */
  import type { Artwork } from "$lib/types";

  import Thumbnail from "./Thumbnail.svelte";

  let { artwork }: { artwork: Artwork } = $props();
</script>

<div
  class="group relative h-full w-full overflow-hidden rounded-card bg-surface-850
         ring-1 ring-white/5 transition hover:ring-2 hover:ring-accent-500/70"
  title={artwork.filePath}
>
  <Thumbnail owner="artwork" id={artwork.id} size={320} alt={artwork.pid} />

  <!-- 信息条：PID 常显，文件路径悬停时浮出来 -->
  <div
    class="pointer-events-none absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/85 to-transparent
           px-2 pt-8 pb-1.5"
  >
    <div class="flex items-baseline justify-between gap-2">
      <span class="font-mono text-xs font-semibold text-accent-400" title="Pixiv 作品号">
        {artwork.pid}
      </span>
      <span class="shrink-0 text-[10px] tabular-nums text-slate-300">p{artwork.page}</span>
    </div>
    <p
      class="truncate font-mono text-[10px] text-slate-400 opacity-0 transition group-hover:opacity-100"
    >
      {artwork.filePath}
    </p>
  </div>
</div>

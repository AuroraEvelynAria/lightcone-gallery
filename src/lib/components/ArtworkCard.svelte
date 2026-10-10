<script lang="ts">
  /**
   * 作品格子：缩略图 + PID 角标，点击打开检视弹窗（弹窗里才加载原图）。
   *
   * 缩略图地址来自列表自带的 `thumbnailPath`，卡片不额外发 IPC。
   *
   * 根节点刻意用 `<div>` 而不是 `<button>`：卡片上还要放「移除」和选择框两个
   * 按钮，HTML 不允许按钮嵌套按钮（`ImageCard` 也是同样的结构）。
   */
  import type { Artwork } from "$lib/types";

  import ArtworkThumbnail from "./ArtworkThumbnail.svelte";

  let {
    artwork,
    selecting = false,
    selected = false,
    onopen,
    onselecttoggle,
    onremove,
  }: {
    artwork: Artwork;
    /** 选择模式下点卡片是勾选而不是打开 */
    selecting?: boolean;
    selected?: boolean;
    onopen: (artwork: Artwork) => void;
    onselecttoggle: (artwork: Artwork) => void;
    onremove: (artwork: Artwork) => void;
  } = $props();

  /** 二次确认：第一下变成「确认移除？」，再点一下才真移除。 */
  let confirming = $state(false);
</script>

<div
  class="group relative h-full w-full overflow-hidden rounded-card bg-surface-850 transition
         {selected ? 'ring-2 ring-accent-500' : 'ring-1 ring-white/5 hover:ring-2 hover:ring-accent-500/70'}"
  title={artwork.originalPath}
  role="presentation"
  onmouseleave={() => (confirming = false)}
>
  <button
    type="button"
    class="absolute inset-0 h-full w-full cursor-pointer focus:outline-none"
    aria-label={selecting
      ? `${selected ? "取消选择" : "选择"}作品 ${artwork.pid} 第 ${artwork.page} 页`
      : `查看作品 ${artwork.pid} 第 ${artwork.page} 页`}
    onclick={() => (selecting ? onselecttoggle(artwork) : onopen(artwork))}
  >
    <ArtworkThumbnail {artwork} />
  </button>

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

  {#if selecting}
    <!-- 选择框 -->
    <button
      type="button"
      class="absolute top-1.5 left-1.5 grid size-6 cursor-pointer place-items-center rounded-full
             backdrop-blur transition {selected
        ? 'bg-accent-500 text-on-accent'
        : 'bg-black/55 text-white/80 hover:bg-black/75'}"
      aria-pressed={selected}
      aria-label={selected ? "取消选择" : "选择"}
      onclick={() => onselecttoggle(artwork)}
    >
      {#if selected}
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" class="size-3.5">
          <path stroke-linecap="round" stroke-linejoin="round" d="M4.5 12.75l6 6 9-13.5" />
        </svg>
      {/if}
    </button>
  {:else}
    <!-- 移除：只从作品库移除，不动原图 -->
    <button
      type="button"
      class="absolute top-1.5 right-1.5 cursor-pointer rounded-md px-1.5 py-1 text-[10px]
             backdrop-blur transition {confirming
        ? 'bg-rose-600/90 text-white opacity-100'
        : 'bg-black/55 text-white/85 opacity-0 hover:bg-black/75 group-hover:opacity-100 focus-visible:opacity-100'}"
      title={confirming ? "再点一下确认（不会删除原图）" : "从作品库移除（不会删除原图）"}
      aria-label={confirming ? "确认从作品库移除" : "从作品库移除"}
      onclick={() => {
        if (confirming) {
          confirming = false;
          onremove(artwork);
        } else {
          confirming = true;
        }
      }}
    >
      {confirming ? "确认移除？" : "移除"}
    </button>
  {/if}
</div>

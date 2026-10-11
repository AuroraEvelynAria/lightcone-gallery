<script lang="ts">
  /**
   * 作品检视弹窗：光锥 3D 视差 + 鼠标跟随扫光。
   *
   * 视差本体、缩放平移、全屏都在 `ImageViewer` 里 —— 通用图库那个弹窗用的是
   * 同一个内核，两边手感必须一致，所以这里只剩作品自己的一条信息栏。
   */
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { fade } from "svelte/transition";

  import { api } from "$lib/api";
  import { registerBackCloser } from "$lib/backStack.svelte";
  import type { Artwork, IllustDetailView } from "$lib/types";

  import ImageViewer from "./ImageViewer.svelte";

  let {
    artwork,
    onclose,
    onnavigate,
    onfilterartist,
    onfiltertag,
  }: {
    artwork: Artwork;
    onclose: () => void;
    /** 上一张 / 下一张。作品库里翻页要等对应数据块到位，所以是异步的 */
    onnavigate?: (delta: number) => void;
    /** 点画师名：只看这位画师的作品 */
    onfilterartist?: (uid: string, name: string) => void;
    /** 点官方标签：只看带这个标签的作品 */
    onfiltertag?: (tag: string) => void;
  } = $props();

  /** Pixiv 侧的信息（标题 / 画师 / 官方标签）。null 表示还没补全过元数据。 */
  let illust = $state<IllustDetailView | null>(null);
  let failed = $state(false);

  // Android 返回手势：有弹窗在就先关弹窗，而不是退出应用（销毁时自动注销）
  $effect(() => registerBackCloser(onclose));

  $effect(() => {
    const pid = artwork.pid;
    let cancelled = false;
    illust = null;
    failed = false;

    api
      .illustDetail(pid)
      .then((result) => {
        if (!cancelled) illust = result;
      })
      .catch(() => {
        // 元数据拿不到不该影响看图，静默降级成「还没取过」
        if (!cancelled) failed = true;
      });

    return () => {
      cancelled = true;
    };
  });
</script>

<div
  class="viewer-shell fixed inset-0 z-50 flex flex-col backdrop-blur-sm"
  role="dialog"
  aria-modal="true"
  aria-label={`作品 ${artwork.pid} 第 ${artwork.page} 页`}
  transition:fade={{ duration: 160 }}
>
  <!-- 弹窗壳是 fixed inset-0 顶到状态栏底下：header 上边距补 safe-area -->
  <header
    class="relative z-10 flex items-center gap-3 px-5 py-3 pt-[calc(0.75rem+env(safe-area-inset-top))] text-soft"
  >
    <span class="font-mono text-sm font-semibold text-accent-400" title="Pixiv 作品号">
      {artwork.pid}
    </span>
    <span class="text-xs text-muted">p{artwork.page}</span>
    {#if illust}
      <span class="truncate text-sm text-fg" data-selectable>{illust.title}</span>
      <span class="shrink-0 text-xs text-muted">
        {illust.artistName}{illust.artistAccount ? ` @${illust.artistAccount}` : ""}
      </span>
    {:else if !failed}
      <span class="text-xs text-mark">读取元数据…</span>
    {/if}
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

  {#if illust && illust.tags.length > 0}
    <!-- 官方标签：只在取到元数据时出现，不占固定高度 -->
    <div
      class="relative z-10 flex flex-wrap items-center gap-1.5 border-b border-surface-800/60 px-5 py-2"
    >
      {#each illust.tags as tag (tag)}
        <button
          type="button"
          class="cursor-pointer rounded-full border border-surface-700 bg-surface-900/60 px-2 py-0.5 text-[11px] text-soft transition hover:border-accent-500 hover:text-accent-400"
          title="只看带「{tag}」的作品"
          onclick={() => onfiltertag?.(tag)}
        >
          {tag}
        </button>
      {/each}
    </div>
  {/if}

  <div class="relative min-h-0 flex-1">
    <ImageViewer
      src={convertFileSrc(artwork.originalPath)}
      alt={`${artwork.pid} p${artwork.page}`}
      failurePath={artwork.originalPath}
      copyPath={artwork.originalPath}
      {onclose}
      {onnavigate}
    >
      <!-- 照片下面展示 Pixiv 信息，不展示本地文件名（理由见 Lightbox.svelte） -->
      {#snippet meta()}
        <span class="viewer-chip">p{artwork.page}</span>
        {#if illust}
          {@const info = illust}
          <span class="viewer-caption" data-selectable>{info.title}</span>
          <button
            type="button"
            class="viewer-artist"
            title="只看 {info.artistName} 的作品"
            onclick={() => onfilterartist?.(info.artistUid, info.artistName)}
          >
            {info.artistName}
          </button>
          {#if info.artistAccount}
            <span class="viewer-caption">@{info.artistAccount}</span>
          {/if}
        {:else}
          <span class="viewer-caption">还没取过元数据（PID {artwork.pid}）</span>
        {/if}
      {/snippet}
    </ImageViewer>
  </div>
</div>

<style lang="postcss">
  @reference "../../app.css";

  /*
    背幕跟着主题走，不写死黑色。

    写死 `bg-black/88` 时，浅色主题下整个弹窗的文字都是深色 —— 深字压深底，
    读不出来。主人用的就是浅色主题（COLD FRONT light），抱怨的正是这个。
    弹窗里的头部、标签、信息行全部用主题色，背幕也必须跟着，否则它们没救了。
  */
  .viewer-shell {
    background: color-mix(in oklab, var(--tone-bg) 88%, transparent);
  }
</style>

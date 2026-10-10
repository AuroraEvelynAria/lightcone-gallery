<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";

  import { api } from "$lib/api";
  import { gallery } from "$lib/gallery.svelte";
  import { formatBytes, formatDate, formatDimensions } from "$lib/format";
  import type { Image, ImageDetail } from "$lib/types";

  let {
    image,
    onclose,
    onnavigate,
  }: {
    image: Image;
    onclose: () => void;
    onnavigate: (delta: number) => void;
  } = $props();

  let detail = $state<ImageDetail | null>(null);
  let previewFailed = $state(false);

  // 切换图片时重新拉取详情（标签 / 相册归属）
  $effect(() => {
    const id = image.id;
    let cancelled = false;
    detail = null;
    previewFailed = false;

    api
      .getImage(id)
      .then((result) => {
        if (!cancelled) detail = result;
      })
      .catch((err) => {
        if (!cancelled) gallery.error = String(err);
      });

    return () => {
      cancelled = true;
    };
  });

  let assignedTagIds = $derived(new Set((detail?.tags ?? []).map((tag) => tag.id)));
  let availableTags = $derived(gallery.tags.filter((tag) => !assignedTagIds.has(tag.id)));

  async function attachTag(tagId: number): Promise<void> {
    try {
      await api.addTagToImage(image.id, tagId);
      detail = await api.getImage(image.id);
      await gallery.refreshSidebar();
    } catch (err) {
      gallery.error = String(err);
    }
  }

  async function detachTag(tagId: number): Promise<void> {
    try {
      await api.removeTagFromImage(image.id, tagId);
      detail = await api.getImage(image.id);
      await gallery.refreshSidebar();
    } catch (err) {
      gallery.error = String(err);
    }
  }

  async function addToAlbum(albumId: number): Promise<void> {
    try {
      await api.addImagesToAlbum(albumId, [image.id]);
      detail = await api.getImage(image.id);
      await gallery.refreshSidebar();
      gallery.notice = "已加入相册";
    } catch (err) {
      gallery.error = String(err);
    }
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") onclose();
    else if (event.key === "ArrowLeft") onnavigate(-1);
    else if (event.key === "ArrowRight") onnavigate(1);
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="fixed inset-0 z-50 flex bg-black/85 backdrop-blur-sm">
  <!-- 点击空白处关闭 -->
  <button type="button" class="absolute inset-0 cursor-default" aria-label="关闭" onclick={onclose}></button>

  <div class="relative z-10 flex min-h-0 flex-1">
    <!-- 预览区 -->
    <div class="relative flex min-w-0 flex-1 items-center justify-center p-4">
      {#if previewFailed}
        <p class="text-sm text-slate-400">无法预览该文件（源文件可能已被移动或删除）</p>
      {:else}
        <img
          src={convertFileSrc(image.path)}
          alt={image.filename}
          draggable="false"
          class="max-h-full max-w-full rounded-lg object-contain shadow-2xl"
          onerror={() => (previewFailed = true)}
        />
      {/if}

      <button
        type="button"
        class="nav-btn left-3"
        aria-label="上一张"
        onclick={() => onnavigate(-1)}
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-5">
          <path stroke-linecap="round" stroke-linejoin="round" d="M15.75 19.5L8.25 12l7.5-7.5" />
        </svg>
      </button>
      <button type="button" class="nav-btn right-3" aria-label="下一张" onclick={() => onnavigate(1)}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-5">
          <path stroke-linecap="round" stroke-linejoin="round" d="M8.25 4.5l7.5 7.5-7.5 7.5" />
        </svg>
      </button>
    </div>

    <!-- 信息面板 -->
    <div class="flex w-80 shrink-0 flex-col border-l border-surface-800 bg-surface-900">
      <div class="flex items-start justify-between gap-2 border-b border-surface-800 p-4">
        <div class="min-w-0">
          <p class="truncate text-sm font-semibold text-slate-100" data-selectable>{image.filename}</p>
          <p class="text-[11px] text-slate-500">{image.extension.toUpperCase()}</p>
        </div>
        <button type="button" class="icon-btn" aria-label="关闭" onclick={onclose}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
            <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <div class="min-h-0 flex-1 space-y-5 overflow-y-auto p-4">
        <!-- 收藏与评分 -->
        <div class="flex items-center gap-3">
          <button
            type="button"
            class="flex cursor-pointer items-center gap-1.5 rounded-md border border-surface-700 px-2.5 py-1 text-xs transition hover:border-surface-600"
            onclick={() => gallery.toggleFavorite(image)}
          >
            <svg
              viewBox="0 0 24 24"
              class="size-4"
              fill={image.isFavorite ? "currentColor" : "none"}
              stroke="currentColor"
              stroke-width="1.8"
              class:text-rose-400={image.isFavorite}
            >
              <path
                stroke-linecap="round"
                stroke-linejoin="round"
                d="M21 8.25c0-2.485-2.099-4.5-4.688-4.5-1.935 0-3.597 1.126-4.312 2.733-.715-1.607-2.377-2.733-4.313-2.733C5.1 3.75 3 5.765 3 8.25c0 7.22 9 12 9 12s9-4.78 9-12z"
              />
            </svg>
            <span class="text-slate-300">{image.isFavorite ? "已收藏" : "收藏"}</span>
          </button>

          <div class="flex items-center gap-0.5">
            {#each [1, 2, 3, 4, 5] as star (star)}
              <button
                type="button"
                class="cursor-pointer p-0.5"
                aria-label={`评 ${star} 星`}
                onclick={() => gallery.setRating(image, star === image.rating ? 0 : star)}
              >
                <svg
                  viewBox="0 0 24 24"
                  class="size-4"
                  fill={star <= image.rating ? "currentColor" : "none"}
                  stroke="currentColor"
                  stroke-width="1.5"
                  class:text-amber-400={star <= image.rating}
                  class:text-slate-600={star > image.rating}
                >
                  <path
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    d="M11.48 3.499a.562.562 0 011.04 0l2.125 5.111a.563.563 0 00.475.345l5.518.442c.499.04.701.663.321.988l-4.204 3.602a.563.563 0 00-.182.557l1.285 5.385a.562.562 0 01-.84.61l-4.725-2.885a.563.563 0 00-.586 0L6.982 20.54a.562.562 0 01-.84-.61l1.285-5.386a.563.563 0 00-.182-.557L3.04 10.385a.562.562 0 01.321-.988l5.518-.442a.563.563 0 00.475-.345L11.48 3.5z"
                  />
                </svg>
              </button>
            {/each}
          </div>
        </div>

        <!-- 标签 -->
        <section>
          <h3 class="panel-head">标签</h3>
          <div class="flex flex-wrap gap-1.5">
            {#each detail?.tags ?? [] as tag (tag.id)}
              <span class="tag-chip">
                {tag.name}
                <button type="button" class="cursor-pointer opacity-60 hover:opacity-100" onclick={() => detachTag(tag.id)}>×</button>
              </span>
            {:else}
              <span class="text-xs text-slate-500">未添加标签</span>
            {/each}
          </div>
          {#if availableTags.length > 0}
            <select
              class="mt-2 w-full cursor-pointer rounded-md border border-surface-700 bg-surface-850 px-2 py-1 text-xs text-slate-300 outline-none focus:border-accent-500"
              onchange={(event) => {
                const value = Number((event.currentTarget as HTMLSelectElement).value);
                (event.currentTarget as HTMLSelectElement).value = "";
                if (value) void attachTag(value);
              }}
            >
              <option value="">＋ 添加标签…</option>
              {#each availableTags as tag (tag.id)}
                <option value={tag.id}>{tag.name}</option>
              {/each}
            </select>
          {/if}
        </section>

        <!-- 所属相册 -->
        <section>
          <h3 class="panel-head">所属相册</h3>
          <div class="flex flex-wrap gap-1.5">
            {#each detail?.albums ?? [] as album (album.id)}
              <span class="tag-chip">{album.name}</span>
            {:else}
              <span class="text-xs text-slate-500">未加入相册</span>
            {/each}
          </div>
          {#if gallery.albums.length > 0}
            <select
              class="mt-2 w-full cursor-pointer rounded-md border border-surface-700 bg-surface-850 px-2 py-1 text-xs text-slate-300 outline-none focus:border-accent-500"
              onchange={(event) => {
                const value = Number((event.currentTarget as HTMLSelectElement).value);
                (event.currentTarget as HTMLSelectElement).value = "";
                if (value) void addToAlbum(value);
              }}
            >
              <option value="">＋ 加入相册…</option>
              {#each gallery.albums as album (album.id)}
                <option value={album.id}>{album.name}</option>
              {/each}
            </select>
          {/if}
        </section>

        <!-- 文件信息 -->
        <section>
          <h3 class="panel-head">文件信息</h3>
          <dl class="space-y-1.5 text-xs">
            <div class="flex justify-between gap-3">
              <dt class="text-slate-500">尺寸</dt>
              <dd class="text-right text-slate-300">{formatDimensions(image.width, image.height)}</dd>
            </div>
            <div class="flex justify-between gap-3">
              <dt class="text-slate-500">大小</dt>
              <dd class="text-right text-slate-300">{formatBytes(image.fileSize)}</dd>
            </div>
            <div class="flex justify-between gap-3">
              <dt class="text-slate-500">修改时间</dt>
              <dd class="text-right text-slate-300">{formatDate(image.modifiedAt)}</dd>
            </div>
            <div class="flex justify-between gap-3">
              <dt class="text-slate-500">加入时间</dt>
              <dd class="text-right text-slate-300">{formatDate(image.indexedAt)}</dd>
            </div>
          </dl>
          <p class="mt-2 rounded-md bg-surface-850 p-2 text-[11px] break-all text-slate-500" data-selectable>
            {image.path}
          </p>
        </section>
      </div>
    </div>
  </div>
</div>

<style lang="postcss">
  @reference "../../app.css";

  .nav-btn {
    @apply absolute cursor-pointer rounded-full bg-black/50 p-2 text-white/80 backdrop-blur
           transition hover:bg-black/70 hover:text-white;
  }
  .panel-head {
    @apply mb-2 text-[11px] font-semibold tracking-wide text-slate-500 uppercase;
  }
  .tag-chip {
    @apply inline-flex items-center gap-1 rounded-full border border-surface-700 px-2 py-0.5
           text-xs text-slate-300;
  }
  .icon-btn {
    @apply shrink-0 cursor-pointer rounded p-1 text-slate-500 transition hover:bg-surface-800 hover:text-slate-200;
  }
</style>

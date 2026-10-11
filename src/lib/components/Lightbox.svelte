<script lang="ts">
  /**
   * 通用图库的检视弹窗：光锥 3D 视差（ImageViewer）+ 右侧信息面板。
   *
   * 整个项目的硬约束是「列表只拉缩略图，原图只在弹窗里出现」，所以这里是原图
   * 唯一会被加载的地方。
   *
   * 全屏时收起右侧面板，把整个窗口让给图片 —— 这也是这个弹窗唯一会变的形态，
   * 所以状态只有 `immersive` 一个布尔值。
   */
  import { convertFileSrc } from "@tauri-apps/api/core";

  import { api } from "$lib/api";
  import { gallery } from "$lib/gallery.svelte";
  import { formatBytes, formatDate, formatDimensions } from "$lib/format";
  import type { IllustDetailView, Image, ImageDetail } from "$lib/types";

  import ImageViewer from "./ImageViewer.svelte";

  let {
    image,
    onclose,
    onnavigate,
    onfilterartist,
    onfiltertag,
  }: {
    image: Image;
    onclose: () => void;
    onnavigate: (delta: number) => void;
    /** 点画师名：只看这位画师的作品 */
    onfilterartist?: (uid: string, name: string) => void;
    /** 点官方标签：只看带这个标签的作品 */
    onfiltertag?: (tag: string) => void;
  } = $props();

  let detail = $state<ImageDetail | null>(null);
  /** Pixiv 侧的信息（标题 / 画师 / 官方标签）。null 表示这条还没补全过元数据。 */
  let illust = $state<IllustDetailView | null>(null);
  /** 全屏时把面板收起来。由 ImageViewer 通知。 */
  let immersive = $state(false);
  /** 手机上右侧信息面板的开关（桌面常驻，此值无效果） */
  let panelOpen = $state(false);

  // 切换图片时重新拉取详情（标签 / 相册归属）与 Pixiv 侧信息
  $effect(() => {
    const id = image.id;
    const pid = image.pid;
    let cancelled = false;
    detail = null;
    illust = null;

    api
      .getImage(id)
      .then((result) => {
        if (!cancelled) detail = result;
      })
      .catch((err) => {
        if (!cancelled) gallery.error = String(err);
      });

    // pid 为 null 表示文件名不符合 `{pid}_p{n}`，那就没有 Pixiv 侧可查
    if (pid) {
      api
        .illustDetail(pid)
        .then((result) => {
          if (!cancelled) illust = result;
        })
        .catch((err) => {
          if (!cancelled) gallery.error = String(err);
        });
    }

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
</script>

<div class="viewer-shell fixed inset-0 z-50 flex backdrop-blur-sm" role="dialog" aria-modal="true">
  <div class="relative min-w-0 flex-1">
    <ImageViewer
      src={convertFileSrc(image.path)}
      alt={image.filename}
      failurePath={image.path}
      copyPath={image.path}
      {onclose}
      {onnavigate}
      onfullscreen={(value) => (immersive = value)}
      onrate={(rating) => gallery.setRating(image, image.rating === rating ? 0 : rating)}
    >
      <!--
        照片下面展示的是**这幅画在 Pixiv 上的信息**，不是本地文件名 ——
        文件名（`148607673_p0.png`）对「这是谁的画、画的是什么」毫无帮助。
        画师名做成按钮，点一下就只看这位画师的作品。
      -->
      {#snippet actions()}
        <!-- 手机上没有常驻面板：ℹ️ 开关信息浮层（桌面面板常驻，不渲染） -->
        <button
          type="button"
          class="cursor-pointer rounded-md p-1.5 text-soft transition hover:text-fg md:hidden"
          aria-label="信息面板"
          onclick={() => (panelOpen = !panelOpen)}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
            <path stroke-linecap="round" stroke-linejoin="round" d="M11.25 11.25l.041-.02a.75.75 0 011.063.852l-.708 2.836a.75.75 0 001.063.853l.041-.021M21 12a9 9 0 11-18 0 9 9 0 0118 0zm-9-3.75h.008v.008H12v-.008z" />
          </svg>
        </button>
      {/snippet}

      {#snippet meta()}
        {#if illust}
          <!-- 收成局部常量：`illust` 是 $state，闭包里读不到外层的收窄 -->
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
        {:else if image.pid}
          <span class="viewer-caption">还没取过元数据（PID {image.pid}）</span>
        {:else}
          <span class="viewer-caption">文件名不符合 {'{pid}_p{n}'} 规范，没有 Pixiv 信息</span>
        {/if}
      {/snippet}
    </ImageViewer>

    <button
      type="button"
      class="absolute top-4 right-4 z-10 cursor-pointer rounded-full bg-black/50 p-2 text-white/80
             backdrop-blur transition hover:bg-black/70 hover:text-white"
      aria-label="关闭"
      title="关闭 (Esc)"
      onclick={onclose}
    >
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-5">
        <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
      </svg>
    </button>
  </div>

  {#if !immersive}
    <!-- 信息面板 -->
    <!-- 手机：面板默认收起，ℹ️ 按钮开关，从右侧浮出；桌面：常驻右栏 -->
    <div
      class="absolute inset-y-0 right-0 z-20 flex w-[88%] max-w-80 shrink-0 flex-col border-l
             border-surface-800 bg-surface-900 shadow-2xl {panelOpen ? '' : 'hidden'}
             md:static md:z-auto md:w-80 md:max-w-none md:shadow-none"
    >
      <div class="border-b border-surface-800 p-4">
        <p class="truncate text-sm font-semibold text-fg" data-selectable>{image.filename}</p>
        <p class="text-[11px] text-muted">
          {image.extension.toUpperCase()} · {formatDimensions(image.width, image.height)}
        </p>
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
            <span class="text-soft">{image.isFavorite ? "已收藏" : "收藏"}</span>
          </button>

          <div class="flex items-center gap-0.5">
            {#each [1, 2, 3, 4, 5] as star (star)}
              <button
                type="button"
                class="cursor-pointer p-1 transition-transform active:scale-90"
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
                  class:text-mark={star > image.rating}
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

        <!-- Pixiv 侧信息：标题 / 画师 / 官方标签 -->
        {#if illust}
          <section>
            <h3 class="panel-head">Pixiv</h3>
            <p class="mb-1 text-sm font-medium text-fg" data-selectable>{illust.title}</p>
            <p class="mb-2 text-xs text-soft">
              {illust.artistName}
              {#if illust.artistAccount}
                <span class="text-muted">@{illust.artistAccount}</span>
              {/if}
              <span class="text-mark">· uid {illust.artistUid}</span>
            </p>
            <div class="flex flex-wrap gap-1.5">
              {#each illust.tags as tag (tag)}
                <button
                  type="button"
                  class="tag-chip cursor-pointer transition hover:bg-accent-500/10 hover:text-accent-400"
                  title="只看带「{tag}」的作品"
                  onclick={() => onfiltertag?.(tag)}
                >
                  {tag}
                </button>
              {/each}
              {#if illust.tags.length === 0}
                <span class="text-xs text-muted">这条作品在 Pixiv 上没有标签</span>
              {/if}
            </div>
          </section>
        {:else if image.pid}
          <section>
            <h3 class="panel-head">Pixiv</h3>
            <p class="text-xs text-muted">
              这条还没取过元数据（PID {image.pid}）。去作品库页点「补全元数据」。
            </p>
          </section>
        {/if}

        <!-- 标签（主人自己打的） -->
        <section>
          <h3 class="panel-head">我的标签</h3>
          <div class="flex flex-wrap gap-1.5">
            {#each detail?.tags ?? [] as tag (tag.id)}
              <span class="tag-chip">
                {tag.name}
                <button
                  type="button"
                  class="cursor-pointer opacity-60 hover:opacity-100"
                  onclick={() => detachTag(tag.id)}>×</button
                >
              </span>
            {:else}
              <span class="text-xs text-muted">未添加标签</span>
            {/each}
          </div>
          {#if availableTags.length > 0}
            <select
              class="mt-2 w-full cursor-pointer rounded-md border border-surface-700 bg-surface-850 px-2 py-1 text-xs text-soft outline-none focus:border-accent-500"
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
              <span class="text-xs text-muted">未加入相册</span>
            {/each}
          </div>
          {#if gallery.albums.length > 0}
            <select
              class="mt-2 w-full cursor-pointer rounded-md border border-surface-700 bg-surface-850 px-2 py-1 text-xs text-soft outline-none focus:border-accent-500"
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
          <dl class="space-y-1.5 font-mono text-xs tabular-nums">
            <div class="flex justify-between gap-3">
              <dt class="text-muted">尺寸</dt>
              <dd class="text-right text-soft">{formatDimensions(image.width, image.height)}</dd>
            </div>
            <div class="flex justify-between gap-3">
              <dt class="text-muted">大小</dt>
              <dd class="text-right text-soft">{formatBytes(image.fileSize)}</dd>
            </div>
            <div class="flex justify-between gap-3">
              <dt class="text-muted">修改时间</dt>
              <dd class="text-right text-soft">{formatDate(image.modifiedAt)}</dd>
            </div>
            <div class="flex justify-between gap-3">
              <dt class="text-muted">加入时间</dt>
              <dd class="text-right text-soft">{formatDate(image.indexedAt)}</dd>
            </div>
          </dl>
          <p class="mt-2 rounded-md bg-surface-850 p-2 font-mono text-[11px] break-all text-muted" data-selectable>
            {image.path}
          </p>
        </section>

        <p class="border-t border-surface-800 pt-3 text-[11px] leading-relaxed text-mark">
          滚轮缩放 · 双击放大 / 还原 · 放大后拖动平移 · ← → 翻页 · F 全屏 · Esc 退出
        </p>
      </div>
    </div>
  {/if}
</div>

<style lang="postcss">
  @reference "../../app.css";

  /* 背幕跟着主题走 —— 理由见 ArtworkLightbox 里的同名样式 */
  .viewer-shell {
    background: color-mix(in oklab, var(--tone-bg) 88%, transparent);
  }

  .panel-head {
    @apply mb-2 text-[11px] font-semibold tracking-wide text-muted uppercase;
  }
  .tag-chip {
    /* 无边框胶囊：半透明底贴毛玻璃面板，hover 用底色而不是描边呼应 */
    @apply inline-flex items-center gap-1 rounded-full bg-surface-850/80 px-2.5 py-0.5
           text-xs text-soft;
  }
</style>

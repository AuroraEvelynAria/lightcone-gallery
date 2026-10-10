<script lang="ts">
  import { api } from "$lib/api";
  import { gallery, type Selection } from "$lib/gallery.svelte";
  import { formatBytes } from "$lib/format";

  let creating = $state<"album" | "tag" | null>(null);
  let draftName = $state("");

  function isActive(target: Selection): boolean {
    const current = gallery.selection;
    if (target.kind === "all" || target.kind === "favorites") {
      return current.kind === target.kind;
    }
    if (current.kind !== target.kind) return false;
    return current.id === target.id;
  }

  async function submitDraft(): Promise<void> {
    const name = draftName.trim();
    if (!name || !creating) return;
    try {
      if (creating === "album") await api.createAlbum(name);
      else await api.createTag(name);
      await gallery.refreshSidebar();
      draftName = "";
      creating = null;
    } catch (err) {
      gallery.error = String(err);
    }
  }

  function cancelDraft(): void {
    creating = null;
    draftName = "";
  }
</script>

<aside class="flex h-full w-64 shrink-0 flex-col border-r border-surface-800 bg-surface-900">
  <div class="flex items-center gap-2 px-4 py-4">
    <div class="grid size-8 place-items-center rounded-lg bg-accent-500/15 text-accent-400">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" class="size-4.5">
        <path
          stroke-linecap="round"
          stroke-linejoin="round"
          d="M6.827 6.175A2.31 2.31 0 015.186 7.23c-.38.054-.757.112-1.134.175C2.999 7.58 2.25 8.507 2.25 9.574V18a2.25 2.25 0 002.25 2.25h15A2.25 2.25 0 0021.75 18V9.574c0-1.067-.75-1.994-1.802-2.169a47.865 47.865 0 00-1.134-.175 2.31 2.31 0 01-1.64-1.055l-.822-1.316a2.192 2.192 0 00-1.736-1.039 48.774 48.774 0 00-5.232 0 2.192 2.192 0 00-1.736 1.039l-.821 1.316z"
        />
        <path stroke-linecap="round" stroke-linejoin="round" d="M16.5 12.75a4.5 4.5 0 11-9 0 4.5 4.5 0 019 0z" />
      </svg>
    </div>
    <div class="min-w-0">
      <p class="truncate text-sm font-semibold text-fg">LightCone Gallery</p>
      <p class="truncate text-[11px] text-muted">
        {gallery.stats ? `${gallery.stats.images} 张图片` : "本地图库"}
      </p>
    </div>
  </div>

  <nav class="flex-1 overflow-y-auto px-2 pb-4">
    <!-- 快捷入口 -->
    <div class="space-y-0.5">
      <button
        type="button"
        class="sidebar-item"
        class:sidebar-item-active={isActive({ kind: "all" })}
        onclick={() => gallery.select({ kind: "all" })}
      >
        <span>全部图片</span>
        {#if gallery.stats}<span class="count">{gallery.stats.images}</span>{/if}
      </button>
      <button
        type="button"
        class="sidebar-item"
        class:sidebar-item-active={isActive({ kind: "favorites" })}
        onclick={() => gallery.select({ kind: "favorites" })}
      >
        <span>收藏</span>
        {#if gallery.stats}<span class="count">{gallery.stats.favorites}</span>{/if}
      </button>

      <!-- 阶段一新增：作品（PID）维度页面 -->
      <a href="/artworks" class="sidebar-item">
        <span>作品库 (PID)</span>
        <span class="count">↗</span>
      </a>

      <!-- 阶段三新增：外观（配色 / 材质 / 明暗） -->
      <a href="/appearance" class="sidebar-item">
        <span>外观</span>
        <span class="count">↗</span>
      </a>
    </div>

    <!-- 图库 -->
    <div class="mt-5">
      <div class="section-head">
        <span>图库</span>
        <button type="button" class="icon-btn" title="添加图库目录" onclick={() => gallery.addLibrary()}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
          </svg>
        </button>
      </div>

      {#each gallery.libraries as library (library.id)}
        <div class="group relative">
          <button
            type="button"
            class="sidebar-item pr-8"
            class:sidebar-item-active={isActive({ kind: "library", id: library.id })}
            title={library.path}
            onclick={() => gallery.select({ kind: "library", id: library.id })}
          >
            <span class="truncate">{library.name}</span>
            <span class="count">{library.imageCount}</span>
          </button>
          <button
            type="button"
            class="icon-btn absolute top-1.5 right-1.5 opacity-0 group-hover:opacity-100"
            title="从图库移除（不会删除磁盘文件）"
            onclick={() => gallery.removeLibrary(library.id)}
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-3.5">
              <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      {:else}
        <p class="px-2.5 py-1 text-xs text-muted">还没有图库</p>
      {/each}
    </div>

    <!-- 相册 -->
    <div class="mt-5">
      <div class="section-head">
        <span>相册</span>
        <button type="button" class="icon-btn" title="新建相册" onclick={() => (creating = "album")}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
          </svg>
        </button>
      </div>

      {#if creating === "album"}
        <form
          class="px-2 py-1"
          onsubmit={(event) => {
            event.preventDefault();
            void submitDraft();
          }}
        >
          <input
            class="draft-input"
            placeholder="相册名称"
            autocomplete="off"
            bind:value={draftName}
            onkeydown={(event) => event.key === "Escape" && cancelDraft()}
            onblur={() => !draftName.trim() && cancelDraft()}
          />
        </form>
      {/if}

      {#each gallery.albums as album (album.id)}
        <button
          type="button"
          class="sidebar-item"
          class:sidebar-item-active={isActive({ kind: "album", id: album.id })}
          onclick={() => gallery.select({ kind: "album", id: album.id })}
        >
          <span class="truncate">{album.name}</span>
          <span class="count">{album.imageCount}</span>
        </button>
      {:else}
        {#if creating !== "album"}
          <p class="px-2.5 py-1 text-xs text-muted">还没有相册</p>
        {/if}
      {/each}
    </div>

    <!-- 标签 -->
    <div class="mt-5">
      <div class="section-head">
        <span>标签</span>
        <button type="button" class="icon-btn" title="新建标签" onclick={() => (creating = "tag")}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
          </svg>
        </button>
      </div>

      {#if creating === "tag"}
        <form
          class="px-2 py-1"
          onsubmit={(event) => {
            event.preventDefault();
            void submitDraft();
          }}
        >
          <input
            class="draft-input"
            placeholder="标签名称"
            autocomplete="off"
            bind:value={draftName}
            onkeydown={(event) => event.key === "Escape" && cancelDraft()}
            onblur={() => !draftName.trim() && cancelDraft()}
          />
        </form>
      {/if}

      <div class="flex flex-wrap gap-1.5 px-2.5 py-1">
        {#each gallery.tags as tag (tag.id)}
          <button
            type="button"
            class="cursor-pointer rounded-full border px-2.5 py-0.5 text-xs transition {isActive({
              kind: 'tag',
              id: tag.id,
            })
              ? 'border-accent-500 bg-accent-500/15 text-accent-400'
              : 'border-surface-700 text-muted hover:border-surface-600'}"
            onclick={() => gallery.select({ kind: "tag", id: tag.id })}
          >
            {tag.name}
            <span class="ml-1 text-[10px] opacity-60">{tag.imageCount}</span>
          </button>
        {:else}
          {#if creating !== "tag"}
            <p class="text-xs text-muted">还没有标签</p>
          {/if}
        {/each}
      </div>
    </div>
  </nav>

  <footer class="border-t border-surface-800 px-3 py-2.5">
    <div class="flex items-center justify-between gap-2 text-[11px] text-muted">
      <span class="truncate">
        缩略图缓存
        {#if gallery.cacheUsage}
          · {formatBytes(gallery.cacheUsage.bytes)} / {gallery.cacheUsage.files} 个文件
        {:else}
          · …
        {/if}
      </span>
      <button
        type="button"
        class="shrink-0 cursor-pointer rounded px-1.5 py-0.5 transition hover:bg-surface-800 hover:text-fg"
        title="删除磁盘上的缩略图缓存，浏览时会按需重建"
        onclick={() => gallery.clearThumbnailCache()}
      >
        清理
      </button>
    </div>
  </footer>
</aside>

<style lang="postcss">
  /* Tailwind v4 中组件的 <style> 是独立编译单元，@apply 需要先 @reference 主题文件 */
  @reference "../../app.css";

  /* 侧栏内部复用的几个组合类，写成 @apply 避免在模板里堆一长串工具类 */
  .sidebar-item {
    @apply flex w-full cursor-pointer items-center justify-between gap-2 rounded-md px-2.5 py-1.5
           text-left text-sm text-muted transition hover:bg-surface-850 hover:text-fg;
  }
  .sidebar-item-active {
    @apply bg-accent-500/15 text-accent-400 hover:bg-accent-500/20 hover:text-accent-400;
  }
  .count {
    @apply shrink-0 text-[11px] tabular-nums text-muted;
  }
  .section-head {
    @apply flex items-center justify-between px-2.5 pb-1 text-[11px] font-semibold
           tracking-wide text-muted uppercase;
  }
  .icon-btn {
    @apply cursor-pointer rounded p-1 text-muted transition hover:bg-surface-800 hover:text-fg;
  }
  .draft-input {
    @apply w-full rounded-md border border-surface-700 bg-surface-850 px-2 py-1 text-sm
           text-fg outline-none focus:border-accent-500;
  }
</style>

<script lang="ts">
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";

  import { api } from "$lib/api";
  import { artworks } from "$lib/artworks.svelte";
  import { registerBackCloser } from "$lib/backStack.svelte";
  import { gallery, type Selection } from "$lib/gallery.svelte";
  import { formatBytes } from "$lib/format";
  import { RATING_ALL_AGES, RATING_NSFW } from "$lib/types";

  let {
    open = true,
    onclose,
  }: {
    /** 手机上抽屉是否拉出（md 以上常驻，此值无效果） */
    open?: boolean;
    /** 任一导航入口被点后回调（主界面收抽屉用） */
    onclose?: () => void;
  } = $props();

  let creating = $state<"album" | "tag" | null>(null);
  let draftName = $state("");

  // ---- Android 选目录面板（自绘，实时 readdir 文件系统） --------------------
  // DocumentsUI 浏览 Pictures 这种上万文件的目录会崩溃循环（HyperOS 缺陷），
  // MediaStore 的索引又滞后且波动（实测同一命令不同时刻分别返回 1518 条和
  // 22 条）—— 干脆自己一层层 readdir，实时又准确。
  const isAndroid = navigator.userAgent.includes("Android");
  let dirPickerOpen = $state(false);
  /** 层级浏览的当前目录（"" = 存储根）。 */
  let currentDir = $state("");
  let listing = $state<{
    subdirs: { name: string; imageCount: number }[];
    directImages: number;
  } | null>(null);

  const fsPath = $derived(`/storage/emulated/0${currentDir}`);
  const currentName = $derived(currentDir.split("/").pop() ?? "");

  // 选目录面板也是一层浮层：按返回先收它，别一路退到桌面
  $effect(() => {
    if (!dirPickerOpen) return;
    return registerBackCloser(() => (dirPickerOpen = false));
  });

  /** 读当前目录的一层列表：子目录（各带直属图片数）+ 直属图片数。 */
  async function loadDir(): Promise<void> {
    listing = null;
    try {
      listing = await api.listSubdirs(fsPath);
    } catch (err) {
      gallery.error = String(err);
      dirPickerOpen = false;
    }
  }

  async function openDirPicker(): Promise<void> {
    dirPickerOpen = true;
    currentDir = "";
    await loadDir();
  }

  /** 进入子目录（文件管理器式浏览）。 */
  async function enterDir(seg: string): Promise<void> {
    currentDir = currentDir === "" ? seg : `${currentDir}/${seg}`;
    await loadDir();
  }

  /** 跳回面包屑的某一级。 */
  async function goDir(path: string): Promise<void> {
    currentDir = path;
    await loadDir();
  }

  /** 把当前浏览到的目录整个收进图库（walkdir 递归，含全部子文件夹）。 */
  async function useCurrentDir(): Promise<void> {
    if (!currentDir) return;
    dirPickerOpen = false;
    await gallery.addLibraryAt(fsPath);
  }

  /** 兜底：调本机文件管理器的 SAF 选目录（Kotlin 桥会优先挑第三方实现）。 */
  async function pickViaFileExplorer(): Promise<void> {
    try {
      const path = await api.androidPickDirectory();
      if (!path) return; // 主人取消
      dirPickerOpen = false;
      await gallery.addLibraryAt(path);
    } catch (err) {
      gallery.error = String(err);
    }
  }

  // 逐种列出来而不是 `current.id === target.id`：画师那一支没有 `id`（用 `uid`），
  // 统一写会让类型收窄失败。列全了还有个好处 —— 以后加新维度时忘了处理会直接报错。
  function isActive(target: Selection): boolean {
    const current = gallery.selection;
    switch (target.kind) {
      case "all":
      case "favorites":
        return current.kind === target.kind;
      case "library":
        return current.kind === "library" && current.id === target.id;
      case "album":
        return current.kind === "album" && current.id === target.id;
      case "tag":
        return current.kind === "tag" && current.id === target.id;
      case "artist":
        return current.kind === "artist" && current.uid === target.uid;
      case "pixivTag":
        return current.kind === "pixivTag" && current.tag === target.tag;
      case "artworks":
        return current.kind === "artworks";
      case "tagGallery":
        // 比 key 而不是比名字：名字理论上唯一（后端也挡重复），但 key 是
        // 「内置 / 自建 + id」拼出来的稳定标识，不依赖名字恰好没被改过
        return current.kind === "tagGallery" && current.key === target.key;
    }
  }

  /**
   * 侧栏内置的两条标签图库。
   *
   * **不落库**：它们就是 `rating` 这一个维度，落库反而要处理「用户把它删了怎么办」。
   * 计数从 `gallery.ratingCounts` 现取。
   */
  const BUILTIN_GALLERIES: { key: string; name: string; rating: number; hint: string }[] = [
    {
      key: "builtin:all-ages",
      name: "全年龄",
      rating: RATING_ALL_AGES,
      hint: "Pixiv 分级为全年龄的作品",
    },
    {
      key: "builtin:nsfw",
      name: "R-18",
      rating: RATING_NSFW,
      hint: "Pixiv 分级为 R-18 / R-18G 的作品（含成人向）",
    },
  ];

  /** 内置图库的条数，从分级计数里取。 */
  function builtinCount(key: string): number | null {
    const counts = gallery.ratingCounts;
    if (!counts) return null;
    return key === "builtin:all-ages" ? counts.allAges : counts.nsfw;
  }

  /** 点一条图库：换成作品库视图并套上它的条件。 */
  function selectGallery(item: { key: string; name: string; tags: string[]; rating: number }): void {
    gallery.select({
      kind: "tagGallery",
      key: item.key,
      name: item.name,
      tags: item.tags,
      rating: item.rating,
    });
  }

  /** 删掉一条自建图库。只去掉侧栏入口，作品一条都不动。 */
  async function deleteGallery(id: number): Promise<void> {
    try {
      await api.deleteTagGallery(id);
      // 删掉的正是当前选中的那条 -> 退回作品库，别停在一条已经不存在的图库上
      if (gallery.selection.kind === "tagGallery" && gallery.selection.key === `custom:${id}`) {
        gallery.select({ kind: "artworks" });
      }
      await gallery.refreshTagGalleries();
    } catch (err) {
      gallery.error = String(err);
    }
  }

  /**
   * 「+」：去作品库把筛选条件配好，再点面板里的「存成图库」。
   *
   * 不在侧栏里再做一个标签选择器 —— 作品库的筛选面板已经有一份带搜索的
   * 标签胶囊（2459 个标签），复制一份只会让两处慢慢长歪。
   */
  function startNewGallery(): void {
    gallery.select({ kind: "artworks" });
    artworks.requestOpenFilter();
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

  /** 删除相册 / 标签：删的是当前选中项就回到全部图片，再刷侧栏。 */
  async function removeAlbum(id: number): Promise<void> {
    try {
      await api.deleteAlbum(id);
      if (gallery.selection.kind === "album" && gallery.selection.id === id) {
        gallery.select({ kind: "all" });
      }
      await gallery.refreshSidebar();
    } catch (err) {
      gallery.error = String(err);
    }
  }

  async function removeTag(id: number): Promise<void> {
    try {
      await api.deleteTag(id);
      if (gallery.selection.kind === "tag" && gallery.selection.id === id) {
        gallery.select({ kind: "all" });
      }
      await gallery.refreshSidebar();
    } catch (err) {
      gallery.error = String(err);
    }
  }

  /** 手机上点进输入框时软键盘会盖住下半屏：键盘弹出（visualViewport 收缩）
      就把输入框滚进可视区。桌面上没有这个 resize，action 无副作用。 */
  function focusVisible(node: HTMLInputElement): { destroy(): void } {
    node.focus();
    const adjust = (): void => node.scrollIntoView({ block: "nearest" });
    window.visualViewport?.addEventListener("resize", adjust, { once: true });
    return {
      destroy(): void {
        window.visualViewport?.removeEventListener("resize", adjust);
      },
    };
  }
</script>

<aside
  class="fixed inset-y-0 left-0 z-40 flex h-full w-64 shrink-0 flex-col border-r border-surface-800
         bg-surface-900 transition-transform duration-200 md:static md:translate-x-0
         {open ? 'translate-x-0' : '-translate-x-full'}"
>
  <!-- 抽屉是 fixed inset-y-0 顶到状态栏底下：顶部标题补 safe-area，
       否则「LightCone Gallery」和系统时钟完全重叠（手机实拍确认） -->
  <div class="flex items-center gap-2 px-4 py-4 pt-[calc(1rem+env(safe-area-inset-top))]">
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

      <!-- 作品（PID）维度：与上面两项同款的原地切换，不再是独立页面 -->
      <button
        type="button"
        class="sidebar-item"
        class:sidebar-item-active={isActive({ kind: "artworks" })}
        onclick={() => gallery.select({ kind: "artworks" })}
      >
        <span>作品库 (PID)</span>
      </button>

      <!-- 阶段三新增：外观（配色 / 材质 / 明暗） -->
      <a href="/appearance" class="sidebar-item">
        <span>外观</span>
        <span class="count">↗</span>
      </a>
    </div>

    <!-- 标签图库：内置两条按 Pixiv 分级（全年龄 / R-18），自建的按「标签 + 分级」。
         存在的理由是把 NSFW 从日常浏览里分出去 —— 主列表行为不变，只是多几个入口 -->
    <div class="mt-5">
      <div class="section-head">
        <span>标签图库</span>
        <button
          type="button"
          class="icon-btn"
          title="新建：先去作品库把筛选条件配好，再点面板里的「存成图库」"
          onclick={startNewGallery}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
          </svg>
        </button>
      </div>

      {#each BUILTIN_GALLERIES as item (item.key)}
        <button
          type="button"
          class="sidebar-item"
          title={item.hint}
          class:sidebar-item-active={isActive({
            kind: "tagGallery",
            key: item.key,
            name: item.name,
            tags: [],
            rating: item.rating,
          })}
          onclick={() =>
            selectGallery({ key: item.key, name: item.name, tags: [], rating: item.rating })}
        >
          <span class="truncate">{item.name}</span>
          <span class="count">{builtinCount(item.key) ?? "…"}</span>
        </button>
      {/each}

      {#each gallery.tagGalleries as item (item.id)}
        <div class="group relative">
          <button
            type="button"
            class="sidebar-item"
            style="padding-right: 2rem"
            title={item.tags.length > 0
              ? `标签：${item.tags.join(" + ")}`
              : "只按分级筛"}
            class:sidebar-item-active={isActive({
              kind: "tagGallery",
              key: `custom:${item.id}`,
              name: item.name,
              tags: item.tags,
              rating: item.rating,
            })}
            onclick={() =>
              selectGallery({
                key: `custom:${item.id}`,
                name: item.name,
                tags: item.tags,
                rating: item.rating,
              })}
          >
            <span class="truncate">{item.name}</span>
            <span class="count">{item.count}</span>
          </button>
          <button
            type="button"
            class="icon-btn absolute top-1.5 right-1.5 opacity-100 md:opacity-0 md:group-hover:opacity-100"
            title="删除这条图库（只是去掉侧栏入口，作品一条都不动）"
            onclick={() => void deleteGallery(item.id)}
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-3.5">
              <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      {:else}
        <p class="px-2.5 py-1 text-xs text-muted">还没有自建图库 —— 点右上角 + 去配条件</p>
      {/each}

      {#if gallery.ratingCounts && gallery.ratingCounts.unknown > 0}
        <p class="px-2.5 py-1 text-[10px] text-mark">
          {gallery.ratingCounts.unknown} 条还没抓元数据，两个分级都不算
        </p>
      {/if}
    </div>

    <!-- 图库：Android 上「+」走 SAF 系统目录选择器（见 gallery.addLibrary 的
         分流），与桌面同一个心智 —— 选一个文件夹，里面的图收进图库 -->
    <div class="mt-5">
      <div class="section-head">
        <span>图库</span>
        <button
          type="button"
          class="icon-btn"
          title="添加图库目录"
          onclick={() => (isAndroid ? void openDirPicker() : gallery.addLibrary())}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
          </svg>
        </button>
      </div>

      {#each gallery.libraries as library (library.id)}
        <div class="group relative">
          <button
            type="button"
            class="sidebar-item" style="padding-right: 2rem"
            class:sidebar-item-active={isActive({ kind: "library", id: library.id })}
            title={library.path}
            onclick={() => gallery.select({ kind: "library", id: library.id })}
          >
            <span class="truncate">{library.name}</span>
            <span class="count">{library.imageCount}</span>
          </button>
          <button
            type="button"
            class="icon-btn absolute top-1.5 right-1.5 opacity-100 md:opacity-0 md:group-hover:opacity-100"
            title="移除图库（数据库记录与缩略图随之清理，不删磁盘文件）"
            onclick={() => void gallery.removeLibrary(library.id)}
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-3.5">
              <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      {:else}
        <p class="px-2.5 py-1 text-xs text-muted">
          {isAndroid ? "还没有图库 —— 点右上角 + 选一个文件夹" : "还没有图库"}
        </p>
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
          class="flex items-center gap-1 px-2 py-1"
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
            use:focusVisible
            onkeydown={(event) => event.key === "Escape" && cancelDraft()}
            onblur={() => !draftName.trim() && cancelDraft()}
          />
          <!-- 手机上没有 Enter：确认 / 取消必须是看得见的按钮 -->
          <button
            type="submit"
            class="icon-btn shrink-0"
            title="确认创建"
            disabled={!draftName.trim()}
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" class="size-4">
              <path stroke-linecap="round" stroke-linejoin="round" d="M4.5 12.75l6 6 9-13.5" />
            </svg>
          </button>
          <button type="button" class="icon-btn shrink-0" title="取消" onclick={cancelDraft}>
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
              <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </form>
      {/if}

      {#each gallery.albums as album (album.id)}
        <div class="group relative">
          <button
            type="button"
            class="sidebar-item" style="padding-right: 2rem"
            class:sidebar-item-active={isActive({ kind: "album", id: album.id })}
            onclick={() => gallery.select({ kind: "album", id: album.id })}
          >
            <span class="truncate">{album.name}</span>
            <span class="count">{album.imageCount}</span>
          </button>
          <!-- 相册只是组织视图，删除不碰图片文件 -->
          <button
            type="button"
            class="icon-btn absolute top-1.5 right-1.5 opacity-100 md:opacity-0 md:group-hover:opacity-100"
            title="删除相册（图片不受影响）"
            onclick={() => void removeAlbum(album.id)}
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-3.5">
              <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
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
          class="flex items-center gap-1 px-2 py-1"
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
            use:focusVisible
            onkeydown={(event) => event.key === "Escape" && cancelDraft()}
            onblur={() => !draftName.trim() && cancelDraft()}
          />
          <button
            type="submit"
            class="icon-btn shrink-0"
            title="确认创建"
            disabled={!draftName.trim()}
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" class="size-4">
              <path stroke-linecap="round" stroke-linejoin="round" d="M4.5 12.75l6 6 9-13.5" />
            </svg>
          </button>
          <button type="button" class="icon-btn shrink-0" title="取消" onclick={cancelDraft}>
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
              <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </form>
      {/if}

      <div class="flex flex-wrap gap-1.5 px-2.5 py-1">
        {#each gallery.tags as tag (tag.id)}
          <!-- 胶囊 = 外层容器（边框/选中态）+ 内部两个按钮：选择区 | 删除区。
               之前删除做成外挂角标，正好压住计数数字（主人抓到）；行内分区
               就没有重叠，删除热区也更合理。 -->
          <div
            class="group/tag inline-flex items-center overflow-hidden rounded-full border text-xs transition
                   {isActive({ kind: 'tag', id: tag.id })
                     ? 'border-accent-500 bg-accent-500/15 text-accent-400'
                     : 'border-surface-700 text-muted'}"
          >
            <button
              type="button"
              class="cursor-pointer py-0.5 pl-2.5 pr-1 transition group-hover/tag:text-fg
                     hover:text-fg"
              onclick={() => gallery.select({ kind: "tag", id: tag.id })}
            >
              {tag.name}
              <span class="ml-1 text-[10px] opacity-60">{tag.imageCount}</span>
            </button>
            <button
              type="button"
              class="cursor-pointer py-0.5 pr-1.5 pl-0.5 text-[11px] leading-none opacity-70 transition
                     hover:text-rose-400 md:opacity-0 md:group-hover/tag:opacity-70"
              title="删除标签（图片不受影响）"
              onclick={() => void removeTag(tag.id)}
            >
              ×
            </button>
          </div>
        {:else}
          {#if creating !== "tag"}
            <p class="text-xs text-muted">还没有标签</p>
          {/if}
        {/each}
      </div>
    </div>
  </nav>

  <!-- 手机抽屉贴到手势条底下：底部补 safe-area（桌面 env() 为 0 无副作用） -->
  <footer class="border-t border-surface-800 px-3 py-2.5 pb-[calc(0.625rem+env(safe-area-inset-bottom))]">
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

{#if dirPickerOpen}
  <!-- Android 选目录面板：底部弹层，列 MediaStore 里全机有图的文件夹。
       自绘而不弹系统 SAF —— DocumentsUI 浏览大目录会崩（HyperOS 缺陷）。 -->
  <div
    class="fixed inset-0 z-50 bg-black/50 md:hidden"
    aria-hidden="true"
    transition:fade={{ duration: 180 }}
    onclick={() => (dirPickerOpen = false)}
  ></div>
  <div
    class="fixed inset-x-0 bottom-0 z-50 flex max-h-[72%] flex-col rounded-t-2xl border-t
           border-surface-700 bg-surface-900 shadow-2xl md:hidden"
    role="dialog"
    aria-label="选择图库目录"
    transition:fly={{ y: 420, duration: 280, easing: cubicOut }}
  >
    <div
      class="flex items-center justify-between border-b border-surface-800 px-4 py-3
             pt-[calc(0.75rem+env(safe-area-inset-top))]"
    >
      <div>
        <p class="text-sm font-semibold text-fg">选择图库目录</p>
        <p class="text-[11px] text-muted">实时浏览手机存储，点进文件夹后收图</p>
      </div>
      <button
        type="button"
        class="cursor-pointer rounded-md p-1.5 text-muted transition hover:bg-surface-850 hover:text-fg"
        aria-label="收起"
        onclick={() => (dirPickerOpen = false)}
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-5">
          <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>
    </div>

    <div class="border-b border-surface-800 px-4 py-2">
      <!-- 面包屑：文件管理器式的层级导航，点任一段跳回那一级 -->
      <div class="flex flex-wrap items-center gap-0.5 text-[11px]">
        <button
          type="button"
          class="cursor-pointer rounded px-1 py-0.5 transition hover:bg-surface-850 {currentDir === ''
            ? 'font-semibold text-accent-400'
            : 'text-muted'}"
          onclick={() => void goDir("")}
        >
          存储根
        </button>
        {#each currentDir.split("/") as seg, index (index)}
          <span class="text-mark">›</span>
          {@const upTo = currentDir.split("/").slice(0, index + 1).join("/")}
          <button
            type="button"
            class="cursor-pointer rounded px-1 py-0.5 transition hover:bg-surface-850 {upTo ===
            currentDir
              ? 'font-semibold text-accent-400'
              : 'text-muted'}"
            onclick={() => void goDir(upTo)}
          >
            {seg}
          </button>
        {/each}
      </div>
    </div>

    <div class="min-h-0 flex-1 overflow-y-auto">
      {#if listing === null}
        <div class="flex items-center justify-center gap-2 py-10 text-sm text-muted">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4 animate-spin">
            <path stroke-linecap="round" d="M12 3a9 9 0 019 9" />
          </svg>
          正在读取目录…
        </div>
      {:else if listing.subdirs.length === 0 && listing.directImages === 0}
        <p class="py-10 text-center text-sm text-muted">这个目录是空的</p>
      {:else}
        {#each listing.subdirs as subdir (subdir.name)}
          <button
            type="button"
            class="flex w-full cursor-pointer items-center justify-between gap-3 px-4 py-2.5
                   text-left transition hover:bg-surface-850"
            onclick={() => void enterDir(subdir.name)}
          >
            <span class="min-w-0 flex-1">
              <span class="block truncate text-sm text-fg">{subdir.name}</span>
              <span class="block truncate text-[10px] text-mark">
                {subdir.imageCount > 0 ? `${subdir.imageCount} 张图片` : "子文件夹"}
              </span>
            </span>
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4 shrink-0 text-mark">
              <path stroke-linecap="round" stroke-linejoin="round" d="M9 5l7 7-7 7" />
            </svg>
          </button>
        {:else}
          {#if listing.directImages === 0}
            <p class="py-6 text-center text-xs text-muted">这个层级没有子文件夹</p>
          {/if}
        {/each}
      {/if}
    </div>

    <!-- 底部操作区：收下当前浏览到的目录 + 兜底的文件管理器入口 -->
    <div class="flex gap-2 border-t border-surface-800 px-4 py-3 pb-[calc(0.75rem+env(safe-area-inset-bottom))]">
      <button
        type="button"
        class="min-w-0 flex-1 cursor-pointer rounded-md bg-accent-600 px-3 py-2 text-xs font-medium
               text-on-accent transition hover:bg-accent-500 disabled:cursor-not-allowed disabled:opacity-50"
        disabled={currentDir === ""}
        title={currentDir === ""
          ? "先在列表里点进一个文件夹"
          : `把 ${fsPath} 及其全部子文件夹收进图库（walkdir 递归）`}
        onclick={() => void useCurrentDir()}
      >
        {currentDir === ""
          ? "进入文件夹后点这里收图"
          : listing && listing.directImages > 0
            ? `收下 ${currentName}（${listing.directImages} 张直属 + 子文件夹）`
            : `收下 ${currentName}`}
      </button>
      <button
        type="button"
        class="shrink-0 cursor-pointer rounded-md border border-surface-700 px-3 py-2 text-xs
               text-soft transition hover:border-surface-600 hover:text-fg"
        title="调本机的文件管理器 App 选目录（SAF 协议）"
        onclick={() => void pickViaFileExplorer()}
      >
        文件管理器…
      </button>
    </div>
  </div>
{/if}

<style lang="postcss">
  /* Tailwind v4 中组件的 <style> 是独立编译单元，@apply 需要先 @reference 主题文件 */
  @reference "../../app.css";

  /* 侧栏内部复用的几个组合类，写成 @apply 避免在模板里堆一长串工具类 */
  .sidebar-item {
    @apply relative flex w-full cursor-pointer items-center justify-between gap-2 rounded-md px-2.5 py-1.5
           text-left text-sm text-muted transition hover:bg-surface-850 hover:text-fg;
  }
  .sidebar-item-active {
    @apply bg-accent-500/15 text-accent-400 hover:bg-accent-500/20 hover:text-accent-400;
  }
  /* 光锥线：选中项左侧的一道竖向强调光。整个应用名字里「LightCone」
     唯一的具象化——比加高亮底色更省、更点题 */
  .sidebar-item-active::before {
    content: "";
    position: absolute;
    left: 0;
    top: 22%;
    bottom: 22%;
    width: 2px;
    border-radius: 9999px;
    background: var(--tone-accent);
    box-shadow: 0 0 8px color-mix(in oklab, var(--tone-accent) 60%, transparent);
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

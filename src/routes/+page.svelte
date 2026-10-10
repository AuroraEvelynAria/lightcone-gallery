<script lang="ts">
  import { onMount } from "svelte";

  import ImageGrid from "$lib/components/ImageGrid.svelte";
  import Lightbox from "$lib/components/Lightbox.svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import Toolbar from "$lib/components/Toolbar.svelte";
  import { gallery } from "$lib/gallery.svelte";
  import type { Image } from "$lib/types";

  let opened = $state<Image | null>(null);
  let noticeTimer: ReturnType<typeof setTimeout> | undefined;

  onMount(() => {
    void gallery.init();
    return () => void gallery.dispose();
  });

  // 操作结果提示 3 秒后自动消失
  $effect(() => {
    if (!gallery.notice) return;
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => (gallery.notice = null), 3000);
    return () => clearTimeout(noticeTimer);
  });

  function navigate(delta: number): void {
    const current = opened;
    if (!current) return;
    const index = gallery.images.findIndex((item) => item.id === current.id);
    const next = index >= 0 ? gallery.images[index + delta] : undefined;
    if (next) opened = next;
  }
</script>

<div class="flex h-screen w-screen overflow-hidden bg-surface-950">
  <Sidebar />

  <div class="flex min-w-0 flex-1 flex-col">
    <Toolbar />
    <main class="min-h-0 flex-1 overflow-y-auto">
      <ImageGrid onopen={(image) => (opened = image)} />
    </main>
  </div>
</div>

{#if opened}
  <Lightbox image={opened} onclose={() => (opened = null)} onnavigate={navigate} />
{/if}

{#if gallery.error || gallery.notice}
  <div class="pointer-events-none fixed bottom-4 left-1/2 z-60 -translate-x-1/2">
    {#if gallery.error}
      <div
        class="pointer-events-auto flex max-w-2xl items-start gap-3 rounded-lg border border-rose-900/60
               bg-rose-950/90 px-4 py-2.5 text-sm text-rose-200 shadow-xl backdrop-blur"
      >
        <span class="min-w-0 break-words">{gallery.error}</span>
        <button
          type="button"
          class="cursor-pointer text-rose-400 transition hover:text-rose-200"
          aria-label="关闭提示"
          onclick={() => (gallery.error = null)}
        >
          ×
        </button>
      </div>
    {:else if gallery.notice}
      <div
        class="pointer-events-auto rounded-lg border border-surface-700 bg-surface-850/95 px-4 py-2.5
               text-sm text-fg shadow-xl backdrop-blur"
      >
        {gallery.notice}
      </div>
    {/if}
  </div>
{/if}

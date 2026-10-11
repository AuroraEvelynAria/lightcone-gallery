<script lang="ts">
  import type { Image } from "$lib/types";
  import { formatBytes } from "$lib/format";

  import Thumbnail from "./Thumbnail.svelte";

  let {
    image,
    onselect,
    ontogglefavorite,
  }: {
    image: Image;
    onselect: (image: Image) => void;
    ontogglefavorite: (image: Image) => void;
  } = $props();
</script>

<div
  class="group relative aspect-square overflow-hidden rounded-card bg-surface-850 ring-1 ring-white/5 transition
         hover:ring-2 hover:ring-accent-500/70 focus-within:ring-2 focus-within:ring-accent-400"
>
  <button
    type="button"
    class="absolute inset-0 h-full w-full cursor-pointer focus:outline-none"
    aria-label={`查看 ${image.filename}`}
    onclick={() => onselect(image)}
  >
    <Thumbnail id={image.id} size={320} alt={image.filename} />
  </button>

  <div
    class="pointer-events-none absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/85 to-transparent
           p-2 pt-8 text-left opacity-0 transition group-hover:opacity-100"
  >
    <p class="truncate text-xs font-medium text-white">{image.filename}</p>
    <p class="truncate text-[10px] text-soft">
      {formatBytes(image.fileSize)}
      {#if image.width && image.height}
        · {image.width}×{image.height}
      {/if}
    </p>
  </div>

  <button
    type="button"
    class="absolute top-1.5 right-1.5 cursor-pointer rounded-full bg-black/55 p-1.5 backdrop-blur
           transition hover:bg-black/75 {image.isFavorite
             ? 'opacity-100'
             : 'opacity-0 group-hover:opacity-100 max-md:opacity-100'}"
    aria-label={image.isFavorite ? "取消收藏" : "加入收藏"}
    onclick={(event) => {
      event.stopPropagation();
      ontogglefavorite(image);
    }}
  >
    <svg
      viewBox="0 0 24 24"
      class="size-4 {image.isFavorite ? 'text-rose-400' : 'text-white'}"
      fill={image.isFavorite ? "currentColor" : "none"}
      stroke="currentColor"
      stroke-width="1.8"
    >
      <path
        stroke-linecap="round"
        stroke-linejoin="round"
        d="M21 8.25c0-2.485-2.099-4.5-4.688-4.5-1.935 0-3.597 1.126-4.312 2.733-.715-1.607-2.377-2.733-4.313-2.733C5.1 3.75 3 5.765 3 8.25c0 7.22 9 12 9 12s9-4.78 9-12z"
      />
    </svg>
  </button>
</div>

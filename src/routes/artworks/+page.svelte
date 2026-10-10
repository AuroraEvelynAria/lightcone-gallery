<script lang="ts">
  /**
   * 旧入口重定向：作品库自 2026-10-11 起并入主界面 —— 侧栏点「作品库 (PID)」
   * 原地切换，与「全部图片 / 收藏」同一套交互，不再有独立的页面和右上角返回按钮。
   * 这里只兼容旧链接 / 旧习惯：选中作品库维度后回主界面。
   */
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";

  import { gallery } from "$lib/gallery.svelte";

  onMount(() => {
    gallery.select({ kind: "artworks" });
    void goto("/", { replaceState: true });
  });
</script>

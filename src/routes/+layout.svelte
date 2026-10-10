<script lang="ts">
  import "../app.css";

  import { onMount } from "svelte";

  import { theme } from "$lib/theme.svelte";

  let { children } = $props();

  onMount(() => {
    // 读回数据库里的权威设置，并接上系统明暗偏好
    void theme.init();
  });

  // 配色 / 材质 / 明暗一变，就把令牌写到 DOM、把材质下发给窗口
  $effect(() => {
    theme.apply();
  });
</script>

{@render children()}

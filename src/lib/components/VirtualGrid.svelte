<script lang="ts">
  /**
   * 定高网格虚拟滚动。
   *
   * 只把视口内的行渲染成真实 DOM，用一块总高度占位撑住滚动条比例 ——
   * 十万条数据也只有几十个节点。纯 CSS + 常量几何，没有 Three.js / WebGL，
   * 也没有任何滚动库依赖。
   *
   * 能直接用公式算而不用逐格测量，是因为格子尺寸只由「列数 × 容器宽度」决定，
   * 与格子里的内容无关。`aspect` 固定了这个前提；要换行高不齐的瀑布流得另写组件。
   *
   * 几何全部落在 `$lib/virtualGeometry` 的纯函数里，那边有独立的测试脚本。
   */
  import type { Snippet } from "svelte";

  import { layoutGrid } from "$lib/virtualGeometry";

  let {
    total,
    children,
    minColumnWidth = 180,
    gap = 12,
    aspect = 1,
    padding = 16,
    overscan = 2,
    onrange,
  }: {
    /** 数据总条数（不是已加载条数）。 */
    total: number;
    /** 渲染第 index 条的内容；数据未就绪时由调用方自己渲染占位。 */
    children: Snippet<[number]>;
    /** 单列最小宽度，用来决定列数。 */
    minColumnWidth?: number;
    gap?: number;
    /** 单元格高宽比，1 为正方。 */
    aspect?: number;
    /** 容器内边距。 */
    padding?: number;
    /** 视口外额外渲染的行数，抵消快速滚动时的白屏。 */
    overscan?: number;
    /** 可视索引区间变化时回调，用于按需拉数据。 */
    onrange?: (start: number, end: number) => void;
  } = $props();

  let scroller = $state<HTMLElement | null>(null);
  let scrollTop = $state(0);
  let contentWidth = $state(0);
  let viewportHeight = $state(0);

  let layout = $derived(
    layoutGrid({
      total,
      contentWidth,
      viewportHeight,
      scrollTop,
      minColumnWidth,
      gap,
      aspect,
      padding,
      overscan,
    }),
  );

  /** 可视区索引，起点对齐行首、终点对齐行尾，正好铺满整行。 */
  let visible = $derived.by(() => {
    const indices: number[] = [];
    for (let index = layout.start; index < layout.end; index += 1) indices.push(index);
    return indices;
  });

  // 列数随容器宽度变，所以监听容器本身而不是 window.resize
  $effect(() => {
    const node = scroller;
    if (!node) return;

    const measure = () => {
      contentWidth = node.clientWidth - padding * 2;
      viewportHeight = node.clientHeight;
    };
    measure();

    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  });

  // 可视区间变化 -> 通知调用方按需取数。虚拟滚动只渲染可见行，
  // 「现在需要哪些索引」这件事只有这里知道。
  $effect(() => {
    if (layout.end <= layout.start) return;
    onrange?.(layout.start, layout.end);
  });

  function onscroll(event: Event): void {
    scrollTop = (event.currentTarget as HTMLElement).scrollTop;
  }
</script>

<div bind:this={scroller} {onscroll} class="h-full overflow-x-hidden overflow-y-auto">
  <div class="relative w-full" style="height: {layout.totalHeight}px">
    <!-- 索引作 key：窗口平移时留在窗口里的格子会复用，不会重复请求缩略图 -->
    <div
      class="absolute grid"
      style="top: {layout.windowTop}px; left: {padding}px; right: {padding}px;
             grid-template-columns: repeat({layout.columns}, minmax(0, 1fr)); gap: {gap}px;"
    >
      {#each visible as index (index)}
        <!-- data-index：虚拟滚动的 DOM 没有位置语义，这个属性是唯一能把节点和数据下标对上的凭据 -->
        <!-- 高度写死：行高公式必须与真实布局一致，否则滚动条会抖 -->
        <div data-index={index} style="height: {layout.cellHeight}px">
          {@render children(index)}
        </div>
      {/each}
    </div>
  </div>
</div>

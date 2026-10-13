<script lang="ts">
  /**
   * 瀑布流虚拟滚动。
   *
   * 与 `VirtualGrid` 的区别只有摆放方式：那边是定高行，这边每一格按自己的高宽比
   * 占位、插进当前最矮的那一列 —— 于是原图的长宽比被完整保留（所见即所得）。
   * 共同点是**都只把可见的那几十格渲染成真实 DOM**：6000 条数据也只有几十个节点。
   *
   * 几何全在 `$lib/masonryGeometry` 的纯函数里（那边有独立的自检脚本，覆盖列高
   * 累加、极端长图、可见区间边界这些「错了也不报错、只是看着不对」的地方）。
   * 本组件只负责：量容器 → 算布局 → 按滚动位置取可见下标 → 绝对定位摆上去。
   *
   * `aspects` 的长度必须等于 `total`。瀑布流的总高取决于全部比例，取不全就只能
   * 边滚边涨 —— 调用方要先把比例表拿到手再挂载本组件（见 `ArtworksView`）。
   */
  import type { Snippet } from "svelte";

  import { masonryFor, visibleMasonryIndices } from "$lib/masonryGeometry";

  let {
    total,
    aspects,
    children,
    minColumnWidth = 150,
    gap = 12,
    padding = 16,
    overscan = 400,
    initialIndex = 0,
    onrange,
  }: {
    /** 数据总条数。 */
    total: number;
    /** 每一项的高宽比（高 / 宽），长度必须等于 `total`。 */
    aspects: number[];
    /** 渲染第 index 条的内容；数据未就绪时由调用方自己渲染占位。 */
    children: Snippet<[number]>;
    /** 单列最小宽度，与定高网格取同一个值，切换排版时格子才不会忽大忽小。 */
    minColumnWidth?: number;
    gap?: number;
    padding?: number;
    /** 视口外上下多渲染多少**像素**，抵消快速滚动时的白屏。 */
    overscan?: number;
    /**
     * 首次布局后把第几条滚到顶部。
     *
     * 切换排版（网格 ↔ 瀑布流）会换掉整个组件，滚动位置自然归零 —— 在几千条里
     * 翻到一半时被弹回顶部很难受。调用方把「当前看的是第几条」传进来，这里用
     * 瀑布流自己的几何算出该滚到哪。之后滚动位置就归组件自己管。
     */
    initialIndex?: number;
    /** 可视索引区间变化时回调，用于按需拉数据。 */
    onrange?: (start: number, end: number) => void;
  } = $props();

  let scroller = $state<HTMLElement | null>(null);
  let scrollTop = $state(0);
  let contentWidth = $state(0);
  let viewportHeight = $state(0);

  let layout = $derived(masonryFor(aspects, { contentWidth, minColumnWidth, gap, padding }));

  let visible = $derived(visibleMasonryIndices(layout, scrollTop, viewportHeight, overscan));

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

  // 首帧量完、布局算出来之后才跳 —— 瀑布流的 top 依赖列宽，列宽又依赖容器宽度。
  // `appliedInitial` 保证只认第一次，之后不再抢用户的方向。
  let appliedInitial = false;
  $effect(() => {
    const node = scroller;
    if (!node || appliedInitial || viewportHeight === 0) return;
    const placement = layout.placements[initialIndex];
    if (!placement) return;
    appliedInitial = true;
    if (initialIndex <= 0) return;
    node.scrollTop = Math.max(0, placement.top - padding);
  });

  // 可见区间变化 -> 通知调用方按需取数。虚拟滚动只渲染可见格，
  // 「现在需要哪些索引」这件事只有这里知道。
  //
  // 传的是首尾下标而不是整份集合：`artworks.ensure` 是按「块」取的，
  // 中间没渲染到的那些迟早会被覆盖到，多取一块远比漏取一块便宜。
  $effect(() => {
    if (visible.length === 0) return;
    onrange?.(visible[0], visible[visible.length - 1] + 1);
  });

  function onscroll(event: Event): void {
    scrollTop = (event.currentTarget as HTMLElement).scrollTop;
  }

  /** 一格的绝对定位。列宽由布局统一给，行位置逐格自己算。 */
  function cellStyle(index: number): string {
    const placement = layout.placements[index];
    const left = padding + placement.column * (layout.columnWidth + gap);
    return `position:absolute;top:${placement.top}px;left:${left}px;width:${layout.columnWidth}px;height:${placement.height}px;`;
  }
</script>

<div bind:this={scroller} {onscroll} class="h-full overflow-x-hidden overflow-y-auto">
  <div class="relative w-full" style="height: {layout.totalHeight}px">
    <!-- 索引作 key：窗口平移时留在窗口里的格子会复用，不会重复请求缩略图 -->
    {#each visible as index (index)}
      <!-- data-index：绝对定位的 DOM 没有位置语义，这个属性是唯一能把节点和数据下标对上的凭据 -->
      <!-- 高度由几何算出来写死：卡片高度必须与布局一致，否则滚动条会抖 -->
      <div data-index={index} data-column={layout.placements[index].column} style={cellStyle(index)}>
        {@render children(index)}
      </div>
    {/each}
  </div>
</div>

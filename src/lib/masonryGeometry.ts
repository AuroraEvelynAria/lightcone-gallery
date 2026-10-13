/**
 * 瀑布流的几何推导（纯函数，不碰 DOM）。
 *
 * 与定高网格（`virtualGeometry`）的区别只有一条：**行不对齐**。每张卡片按自己的
 * 高宽比占位，摆进当前最矮的那一列 —— 于是原图的长宽比被完整保留（所见即所得），
 * 代价是「第 i 张落在第几列、top 是多少」必须把全部比例先过一遍才算得出来。
 *
 * 这一点也决定了虚拟化的做法不同：定高网格能 O(1) 从 scrollTop 推出可见区间，
 * 瀑布流只能**按列二分**（每列内部天然按 top 升序，因为是顺序追加的）。
 * 两边渲染的节点数仍是同一个量级 —— 都只有可见的那几十格。
 *
 * 抽成纯函数是为了能脱离浏览器验证：瀑布流最容易出错的地方（列高累加、
 * 极端比例、可见区间边界）都只表现在「格子轻微重叠/漏项/滚动条抖动」上，
 * 靠肉眼几乎测不出来。
 */

// 带 `.ts` 后缀：`tests/` 下的自检脚本由 Node 原生跑，相对导入必须写全扩展名
// （tsconfig 里已开 allowImportingTsExtensions）。Vite 打包时同样认这种写法。
import { columnWidthFor, columnsFor } from "./virtualGeometry.ts";

/**
 * 卡片高宽比的夹取范围（高 / 宽）。
 *
 * 1:5 的长条和 5:1 的横幅如果照实占位，一格就能吃掉整屏或者细成一条线；
 * 超出范围的会在框内被 `object-cover` 裁掉一点。本机 4430 张实测：夹到
 * `[0.5, 2.5]` 只有 **2.6%** 的作品需要裁，而正方网格是 100% 全裁。
 *
 * **必须与后端 `artworks::aspect` 的夹取范围一致**，否则前端算的框高与
 * 后端给的比例对不上，格子会错位。
 */
export const MIN_ASPECT = 0.5;
export const MAX_ASPECT = 2.5;

/** 比例读不出来时（NaN / 负数 / Infinity）兜底成这个值，与后端的兜底一致。 */
const FALLBACK_ASPECT = 1.4;

export interface MasonryInput {
  /** 每一项的高宽比（高 / 宽），下标与数据下标一一对应。 */
  aspects: number[];
  columnCount: number;
  columnWidth: number;
  gap: number;
  padding: number;
}

export interface MasonryPlacement {
  column: number;
  /** 卡片顶边的 y 坐标（含 padding 与所有间距）。 */
  top: number;
  height: number;
}

export interface MasonryLayout {
  placements: MasonryPlacement[];
  /** 每一列里按 `top` 升序的**数据下标**。可见区间按列二分要用它。 */
  byColumn: number[][];
  columnWidth: number;
  columnCount: number;
  /** 含 padding 的内容总高，用来撑滚动条。 */
  totalHeight: number;
}

/** 把比例夹进合法范围；NaN / 非正数走兜底值，绝不把脏数据传进高度计算。 */
export function clampAspect(aspect: number): number {
  if (!Number.isFinite(aspect) || aspect <= 0) return FALLBACK_ASPECT;
  return Math.min(MAX_ASPECT, Math.max(MIN_ASPECT, aspect));
}

/**
 * 把一串高宽比排成瀑布流。
 *
 * 顺序敏感：每一项都放进**当前最矮**的那一列（平局取最左）。这样任何一批数据
 * 排出来的结果都是确定的，不会因为是「滚动中重算」还是「首屏」而变。
 *
 * `aspects.length` 就是数据总数；比例表没取全时不要调它（调用方先兜住）。
 */
export function layoutMasonry(input: MasonryInput): MasonryLayout {
  const { aspects, columnWidth, gap, padding } = input;
  const columnCount = Math.max(1, Math.floor(input.columnCount));

  const columnTops = new Array<number>(columnCount).fill(padding);
  const placements: MasonryPlacement[] = new Array(aspects.length);
  const byColumn: number[][] = Array.from({ length: columnCount }, () => []);

  for (let index = 0; index < aspects.length; index += 1) {
    let target = 0;
    for (let column = 1; column < columnCount; column += 1) {
      if (columnTops[column] < columnTops[target]) target = column;
    }

    const height = columnWidth * clampAspect(aspects[index]);
    placements[index] = { column: target, top: columnTops[target], height };
    byColumn[target].push(index);
    columnTops[target] += height + gap;
  }

  // 末尾那一格后面不该再算一个 gap —— 多算会让滚动条比内容长一点，滑到底空一截
  const tallest = aspects.length === 0 ? padding : Math.max(...columnTops) - gap;

  return {
    placements,
    byColumn,
    columnWidth,
    columnCount,
    totalHeight: tallest + padding,
  };
}

/**
 * 可见（含 overscan）的数据下标，按数据顺序返回。
 *
 * 每列内部的下标天然按 `top` 升序（顺序追加的），所以二分出「第一个底边进入
 * 可见带」的那一项，再往后扫到「顶边越过可见带」为止。
 * 复杂度 `O(列数 × log n + 命中数)`，与数据总量无关。
 */
export function visibleMasonryIndices(
  layout: MasonryLayout,
  scrollTop: number,
  viewportHeight: number,
  overscan: number,
): number[] {
  // 先把 scrollTop 夹进有效范围。
  //
  // 容器变宽 -> 列数变少 -> 内容高度缩短时，浏览器**还来不及**把 scrollTop 夹下来
  // （夹紧发生在下一帧），此刻按原值算就会得到一个完全越界的空窗口，表现是
  // 「滚到过深处突然白屏」。定高网格那边对 `firstRow` 封顶也是为同一件事。
  const maxTop = Math.max(0, layout.totalHeight - viewportHeight);
  const clamped = Math.min(Math.max(0, scrollTop), maxTop);

  const bandTop = clamped - overscan;
  const bandBottom = clamped + viewportHeight + overscan;
  const hits: number[] = [];

  for (const column of layout.byColumn) {
    let low = 0;
    let high = column.length;
    while (low < high) {
      const mid = (low + high) >> 1;
      const placement = layout.placements[column[mid]];
      if (placement.top + placement.height < bandTop) low = mid + 1;
      else high = mid;
    }

    for (let cursor = low; cursor < column.length; cursor += 1) {
      const placement = layout.placements[column[cursor]];
      if (placement.top > bandBottom) break;
      hits.push(column[cursor]);
    }
  }

  // 按列收集出来是「列优先」，排回数据顺序：DOM 顺序稳定，虚拟滚动靠 key 复用组件实例
  hits.sort((a, b) => a - b);
  return hits;
}

/** 由容器宽度推出列数与列宽，再排版。列数公式与定高网格共用，切换排版时格子一样大。 */
export function masonryFor(
  aspects: number[],
  viewport: { contentWidth: number; minColumnWidth: number; gap: number; padding: number },
): MasonryLayout {
  const columns = columnsFor(viewport.contentWidth, viewport.minColumnWidth, viewport.gap);
  return layoutMasonry({
    aspects,
    columnCount: columns,
    columnWidth: columnWidthFor(viewport.contentWidth, columns, viewport.gap),
    gap: viewport.gap,
    padding: viewport.padding,
  });
}

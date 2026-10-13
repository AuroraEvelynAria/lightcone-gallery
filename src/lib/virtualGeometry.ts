/**
 * 定高网格的几何推导（纯函数，不碰 DOM）。
 *
 * 抽出来是为了能脱离浏览器验证：虚拟滚动最容易出错的就是
 * 「列数 -> 格宽 -> 行高 -> 可视区间 -> 定位偏移」这条链，
 * 某个环节算错时表现只是滚动条轻微抖动或偶尔白屏，靠肉眼很难发现。
 */

export interface GridInput {
  /** 数据总条数。 */
  total: number;
  /** 容器内容区宽度，已扣掉左右 padding。 */
  contentWidth: number;
  /** 容器可视高度。 */
  viewportHeight: number;
  /** 当前滚动位置。 */
  scrollTop: number;
  /** 单列最小宽度。 */
  minColumnWidth: number;
  /** 格间距。 */
  gap: number;
  /** 单元格高宽比，1 为正方。 */
  aspect: number;
  /** 容器内边距。 */
  padding: number;
  /** 视口外额外渲染的行数。 */
  overscan: number;
}

export interface GridLayout {
  columns: number;
  cellWidth: number;
  cellHeight: number;
  /** 含间距的整行高度，既是相邻行的顶上距离也是占位高度的累加单位。 */
  rowHeight: number;
  rowCount: number;
  totalHeight: number;
  firstRow: number;
  lastRow: number;
  /** 可视窗口顶边的 y 坐标。 */
  windowTop: number;
  /** 需要渲染的绝对索引区间 `[start, end)`，start 对齐到行首。 */
  start: number;
  end: number;
}

/** 格子宽度下限；内容区极窄时不让格子退化成一条线。 */
export const MIN_CELL_WIDTH = 40;

/**
 * 由内容区宽度推列数。
 *
 * **定高网格与瀑布流共用这一份**：两种排版的列数与列宽必须完全一致，
 * 否则切换排版时格子会忽大忽小 —— 明明只是换了摆放方式，观感却像换了密度。
 */
export function columnsFor(contentWidth: number, minColumnWidth: number, gap: number): number {
  return Math.max(1, Math.floor((contentWidth + gap) / (minColumnWidth + gap)));
}

/** 把内容区均分成 `columns` 列之后，每列有多宽。 */
export function columnWidthFor(contentWidth: number, columns: number, gap: number): number {
  return Math.max(MIN_CELL_WIDTH, (contentWidth - (columns - 1) * gap) / columns);
}

export function layoutGrid(input: GridInput): GridLayout {
  const {
    total,
    contentWidth,
    viewportHeight,
    scrollTop,
    minColumnWidth,
    gap,
    aspect,
    padding,
    overscan,
  } = input;

  const columns = columnsFor(contentWidth, minColumnWidth, gap);
  const cellWidth = columnWidthFor(contentWidth, columns, gap);
  const cellHeight = cellWidth * aspect;
  const rowHeight = cellHeight + gap;
  const rowCount = Math.ceil(total / columns);
  const totalHeight = rowCount * rowHeight + padding * 2;

  // 向上取整到行首、向下留 overscan 行；clamp 保证空数据时区间为空
  //
  // firstRow 额外用 rowCount - 1 封顶：容器变宽 -> 行数变少 -> 内容高度缩短时，
  // 浏览器还来不及夹紧 scrollTop，此时若不封顶就会算出一个越界的空窗口，
  // 表现是滚到过深处突然白屏。
  const firstRow = Math.min(
    Math.max(0, rowCount - 1),
    Math.max(0, Math.floor((scrollTop - padding) / rowHeight) - overscan),
  );
  const lastRow = Math.min(
    rowCount - 1,
    Math.ceil((scrollTop + viewportHeight - padding) / rowHeight) + overscan,
  );

  const empty = rowCount === 0 || lastRow < firstRow;

  return {
    columns,
    cellWidth,
    cellHeight,
    rowHeight,
    rowCount,
    totalHeight,
    firstRow,
    lastRow,
    windowTop: padding + firstRow * rowHeight,
    start: empty ? 0 : firstRow * columns,
    // 末尾对齐到行尾：这样渲染出的索引正好铺满整行，网格不会缺角
    end: empty ? 0 : Math.min(total, (lastRow + 1) * columns),
  };
}

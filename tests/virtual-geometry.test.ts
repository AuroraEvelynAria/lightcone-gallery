/**
 * 虚拟滚动几何的自检。
 *
 * 不用测试框架：Node 24 能原生跑 TS，几个断言加退出码就够了，
 * 不值得为此往项目里塞 vitest。
 *
 * 运行：npm test
 */

import assert from "node:assert/strict";

import { layoutGrid, type GridInput } from "../src/lib/virtualGeometry.ts";

/** 基准输入，用例只覆盖关心的字段。 */
function grid(overrides: Partial<GridInput> = {}): GridInput {
  return {
    total: 1000,
    contentWidth: 1000,
    viewportHeight: 800,
    scrollTop: 0,
    minColumnWidth: 200,
    gap: 12,
    aspect: 1,
    padding: 16,
    overscan: 2,
    ...overrides,
  };
}

let failures = 0;

function test(name: string, run: () => void): void {
  try {
    run();
    console.log(`  ✓ ${name}`);
  } catch (error) {
    failures += 1;
    const message = error instanceof Error ? error.message : String(error);
    console.error(`  ✗ ${name}\n      ${message}`);
  }
}

console.log("virtualGeometry");

test("列数由容器宽度推出，格宽正好铺满内容区", () => {
  const layout = layoutGrid(grid());
  // floor((1000 + 12) / (200 + 12)) = 4
  assert.equal(layout.columns, 4);
  assert.equal(layout.cellWidth, (1000 - 3 * 12) / 4);
  // 列宽之和加间距必须精确等于内容区宽度，否则每行会累积误差、滚动条抖动
  assert.equal(layout.columns * layout.cellWidth + (layout.columns - 1) * 12, 1000);
});

test("行高含间距，占位高度装得下最后一行", () => {
  const layout = layoutGrid(grid({ total: 97, aspect: 1.25 }));
  assert.equal(layout.rowCount, Math.ceil(97 / layout.columns));
  assert.equal(layout.cellHeight, layout.cellWidth * 1.25);
  assert.equal(layout.rowHeight, layout.cellHeight + 12);
  assert.equal(layout.totalHeight, layout.rowCount * layout.rowHeight + 32);

  const lastRowBottom = 16 + (layout.rowCount - 1) * layout.rowHeight + layout.cellHeight;
  assert.ok(
    lastRowBottom <= layout.totalHeight - 16,
    `末行溢出占位高度: ${lastRowBottom} > ${layout.totalHeight - 16}`,
  );
});

test("空数据不产生任何索引", () => {
  const layout = layoutGrid(grid({ total: 0 }));
  assert.equal(layout.rowCount, 0);
  assert.equal(layout.start, 0);
  assert.equal(layout.end, 0);
  assert.equal(layout.totalHeight, 32);
});

test("首屏从第 0 条开始且铺满整行", () => {
  const layout = layoutGrid(grid());
  assert.equal(layout.start, 0);
  assert.equal(layout.windowTop, 16);
  assert.equal(layout.end % layout.columns, 0, "终点未对齐到行尾");
});

// 网格布局的前提：窗口起点必须落在行首，否则首行格子会错位到上一行去
test("窗口起点始终对齐到行首", () => {
  for (const scrollTop of [0, 37, 253, 1000, 5000, 123_456]) {
    const layout = layoutGrid(grid({ total: 100_000, scrollTop }));
    assert.equal(
      layout.start % layout.columns,
      0,
      `scrollTop=${scrollTop} 时 start=${layout.start} 未对齐到 ${layout.columns} 列`,
    );
    assert.ok(layout.end <= 100_000);
    assert.ok(
      layout.end % layout.columns === 0 || layout.end === 100_000,
      `scrollTop=${scrollTop} 时终点既没对齐也没截断到总数`,
    );
  }
});

test("滚动位置超出内容长度时回落到末行，不会白屏", () => {
  // 容器变宽 -> 行数变少 -> 内容变矮，浏览器尚未夹紧 scrollTop 的瞬间
  const layout = layoutGrid(grid({ total: 40, scrollTop: 999_999 }));
  assert.ok(layout.end > layout.start, "越界的滚动位置算出了空窗口");
  assert.equal(layout.end, 40, "应回落到末尾并渲染到末条");
});

test("渲染节点数与数据总量解耦", () => {
  // 可视行数 + 上下 overscan + 1 行余量，再乘列数
  const cap = 64;
  for (const total of [0, 10, 1000, 100_000]) {
    for (const scrollTop of [0, 5000, 123_456]) {
      const layout = layoutGrid(grid({ total, scrollTop }));
      assert.ok(
        layout.end - layout.start <= cap,
        `total=${total} scrollTop=${scrollTop} 渲染了 ${layout.end - layout.start} 条，超过上限 ${cap}`,
      );
    }
  }
});

test("连续滚动不漏项", () => {
  const base = grid({ total: 2573, viewportHeight: 800 });
  const first = layoutGrid(base);
  const maxScroll = Math.max(0, first.totalHeight - base.viewportHeight);

  let previousEnd = 0;
  // 步长与行高不整除，用来覆盖错位情形
  for (let scrollTop = 0; scrollTop <= maxScroll; scrollTop += 37) {
    const layout = layoutGrid({ ...base, scrollTop });
    assert.equal(layout.start % layout.columns, 0);
    assert.ok(
      layout.start <= previousEnd,
      `scrollTop=${scrollTop} 处出现空档：窗口从 ${layout.start} 开始，上一个窗口只覆盖到 ${previousEnd}`,
    );
    previousEnd = Math.max(previousEnd, layout.end);
  }

  const bottom = layoutGrid({ ...base, scrollTop: maxScroll });
  assert.equal(bottom.end, base.total, "滚到底应覆盖到末条");
});

test("末页不满时按总数截断", () => {
  // 1001 条在 4 列下是 251 行，末行只有 1 条，该行首条索引为 250 * 4
  const layout = layoutGrid(grid({ total: 1001, scrollTop: 999_999 }));
  assert.equal(layout.start, 250 * 4);
  assert.equal(layout.end, 1001);
});

test("容器极窄时不产生 NaN 或负数", () => {
  for (const contentWidth of [0, 1, 50, 199]) {
    const layout = layoutGrid(grid({ contentWidth }));
    assert.equal(layout.columns, 1, `contentWidth=${contentWidth} 应退化为单列`);
    assert.ok(Number.isFinite(layout.cellWidth) && layout.cellWidth > 0);
    assert.ok(Number.isFinite(layout.rowHeight) && layout.rowHeight > 0);
    assert.ok(Number.isFinite(layout.totalHeight) && layout.totalHeight >= 0);
    assert.ok(layout.start >= 0 && layout.end >= layout.start);
  }
});

if (failures > 0) {
  console.error(`\n${failures} 个用例未通过`);
  process.exitCode = 1;
} else {
  console.log("全部通过");
}

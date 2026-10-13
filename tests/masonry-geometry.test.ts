/**
 * 瀑布流几何的自检。
 *
 * 与 `virtual-geometry.test.ts` 同一套做法：Node 24 原生跑 TS，断言 + 退出码，
 * 不引测试框架。
 *
 * 重点覆盖三类容易静默出错的地方：
 * 1. **列高累加** —— 算错一点只是格子轻微重叠，肉眼要盯着看才发现；
 * 2. **极端比例** —— 长条图会把某一列撑爆，且只在个别图库上出现；
 * 3. **可见区间边界** —— 差一像素就是滚动中偶尔漏一格 / 白一格。
 */

import assert from "node:assert/strict";

import {
  MAX_ASPECT,
  MIN_ASPECT,
  clampAspect,
  layoutMasonry,
  masonryFor,
  visibleMasonryIndices,
  type MasonryInput,
} from "../src/lib/masonryGeometry.ts";

function masonry(overrides: Partial<MasonryInput> = {}): MasonryInput {
  return {
    aspects: Array.from({ length: 60 }, () => 1.5),
    columnCount: 3,
    columnWidth: 160,
    gap: 12,
    padding: 16,
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

console.log("masonryGeometry");

test("每一格都摆进当前最矮的列，且不与同列上一格重叠", () => {
  const layout = layoutMasonry(masonry());

  for (const column of layout.byColumn) {
    for (let i = 1; i < column.length; i += 1) {
      const previous = layout.placements[column[i - 1]];
      const current = layout.placements[column[i]];
      const gap = current.top - (previous.top + previous.height);
      assert.ok(
        Math.abs(gap - 12) < 1e-9,
        `同列相邻两格的间距应正好是 gap，实际 ${gap}`,
      );
    }
  }
});

test("等高比例下各列高度差不超过一格 —— 最短列优先确实在均衡", () => {
  const layout = layoutMasonry(masonry());
  const bottoms = layout.byColumn.map((column) => {
    const last = layout.placements[column[column.length - 1]];
    return last.top + last.height;
  });

  const spread = Math.max(...bottoms) - Math.min(...bottoms);
  assert.ok(
    spread <= 160 * 1.5 + 12,
    `比例全相同时各列应当基本齐平，实际最大差 ${spread}`,
  );
});

test("列高累加正确：总高等于最高那一列的底边（末尾不多算一个 gap）", () => {
  const layout = layoutMasonry(masonry({ aspects: [1, 1, 1, 1] }));
  // 4 项 3 列：前 3 项各占一列，第 4 项落到第 1 列
  // 单格高 = 160，顶上留 padding 16
  const highest = Math.max(
    ...layout.byColumn.map((column) => {
      const last = layout.placements[column[column.length - 1]];
      return last.top + last.height;
    }),
  );
  assert.equal(layout.totalHeight, highest + 16, "总高 = 最高列底边 + 下 padding");
});

test("空数据不产生 NaN，也不留下负高度", () => {
  const layout = layoutMasonry(masonry({ aspects: [] }));
  assert.equal(layout.totalHeight, 32, "只剩上下 padding");
  assert.equal(layout.placements.length, 0);
  assert.ok(layout.byColumn.every((column) => column.length === 0));
});

test("极端长图 / 宽图被夹住，不会一格吃掉整屏", () => {
  assert.equal(clampAspect(8), MAX_ASPECT);
  assert.equal(clampAspect(0.05), MIN_ASPECT);
  assert.equal(clampAspect(1.5), 1.5);

  // 脏数据不能让高度变成 NaN —— 那会把整列的 top 污染成 NaN，全列消失
  for (const bad of [NaN, Infinity, -Infinity, 0, -3]) {
    const value = clampAspect(bad);
    assert.ok(Number.isFinite(value) && value > 0, `clampAspect(${bad}) 应为有限正数`);
  }

  const layout = layoutMasonry(masonry({ aspects: [100, 0.001, 1.5] }));
  for (const placement of layout.placements) {
    assert.ok(
      Number.isFinite(placement.top) && Number.isFinite(placement.height),
      "夹取之后不能出现 NaN",
    );
    assert.ok(placement.height <= 160 * MAX_ASPECT + 1e-9);
    assert.ok(placement.height >= 160 * MIN_ASPECT - 1e-9);
  }
});

test("可见区间不漏项：滚过的范围内每一格都被渲染过", () => {
  const layout = layoutMasonry(masonry());
  const seen = new Set<number>();

  // 一屏 600px，每次滚 100px，扫完整篇
  for (let scrollTop = 0; scrollTop < layout.totalHeight; scrollTop += 100) {
    for (const index of visibleMasonryIndices(layout, scrollTop, 600, 0)) {
      seen.add(index);
    }
  }

  for (let index = 0; index < 60; index += 1) {
    assert.ok(seen.has(index), `第 ${index} 项在整趟滚动里一次都没被渲染`);
  }
});

test("可见区间不越界，也不把远处的东西拉进来", () => {
  const layout = layoutMasonry(masonry());
  const maxTop = Math.max(0, layout.totalHeight - 600);

  for (const scrollTop of [0, 500, 2000, layout.totalHeight - 600, layout.totalHeight]) {
    // 期望值这边也要夹一次：实现会把 scrollTop 夹进有效范围（见上面那条用例），
    // 拿原值算期望就变成在验旧行为了
    const effective = Math.min(Math.max(0, scrollTop), maxTop);
    for (const index of visibleMasonryIndices(layout, scrollTop, 600, 0)) {
      assert.ok(index >= 0 && index < 60, `下标越界: ${index}`);
      const placement = layout.placements[index];
      const overlaps =
        placement.top <= effective + 600 && placement.top + placement.height >= effective;
      assert.ok(
        overlaps,
        `第 ${index} 项与可见带 [${effective}, ${effective + 600}) 不相交`,
      );
    }
  }
});

test("overscan 只放宽边界，不会改变命中顺序", () => {
  const layout = layoutMasonry(masonry());
  const tight = visibleMasonryIndices(layout, 400, 600, 0);
  const loose = visibleMasonryIndices(layout, 400, 600, 300);

  for (const index of tight) assert.ok(loose.includes(index), "放宽后必须仍然包含原来的");
  assert.deepEqual([...loose].sort((a, b) => a - b), loose, "返回顺序必须是数据顺序");
});

test("滚到超出内容高度时回落到末尾，不白屏也不抛错", () => {
  const layout = layoutMasonry(masonry());
  const hits = visibleMasonryIndices(layout, layout.totalHeight + 5000, 600, 0);

  // 这不是假想场景：切排版 / 转屏会让列数变少、内容变矮，而浏览器的 scrollTop
  // 要下一帧才夹紧 —— 那一帧按原值算就会得到一个空窗口，整屏白
  assert.ok(hits.length > 0, "越界滚动时应回落到末尾那一批，而不是整屏空白");
  assert.ok(hits.every((index) => index >= 0 && index < 60));
  assert.ok(
    hits.includes(59),
    "回落到末尾时应至少包含最后一项 —— 否则用户滑到底看到的是空白",
  );
});

test("列数由容器宽度推出，与定高网格用同一套公式", () => {
  // 手机两列：内容区 333.5 CSS、最小列宽 150、间距 12
  const phone = masonryFor(
    Array.from({ length: 10 }, () => 1.5),
    { contentWidth: 333.5, minColumnWidth: 150, gap: 12, padding: 16 },
  );
  assert.equal(phone.columnCount, 2);
  assert.ok(Math.abs(phone.columnWidth - (333.5 - 12) / 2) < 1e-9);

  // 桌面四列：内容区 982 CSS、最小列宽 200
  const desktop = masonryFor(
    Array.from({ length: 10 }, () => 1.5),
    { contentWidth: 982, minColumnWidth: 200, gap: 12, padding: 16 },
  );
  assert.equal(desktop.columnCount, 4);
});

test("数据越多渲染的格子越少 —— 节点数与数据总量解耦", () => {
  const few = layoutMasonry(masonry({ aspects: Array.from({ length: 60 }, () => 1.5) }));
  const many = layoutMasonry(masonry({ aspects: Array.from({ length: 6000 }, () => 1.5) }));

  const visibleFew = visibleMasonryIndices(few, 2000, 800, 300).length;
  const visibleMany = visibleMasonryIndices(many, 2000, 800, 300).length;

  assert.ok(
    visibleMany < visibleFew * 2,
    `数据涨 100 倍，渲染量不该跟着涨：${visibleFew} -> ${visibleMany}`,
  );
});

test("容器极窄时不产生 NaN 或负数", () => {
  for (const contentWidth of [0, 1, 50, 199]) {
    const layout = masonryFor(
      [1.5, 0.6, 2.4],
      { contentWidth, minColumnWidth: 150, gap: 12, padding: 16 },
    );
    assert.equal(layout.columnCount, 1, `contentWidth=${contentWidth} 应退化为单列`);
    assert.ok(Number.isFinite(layout.columnWidth) && layout.columnWidth > 0);
    assert.ok(Number.isFinite(layout.totalHeight) && layout.totalHeight >= 0);
    for (const placement of layout.placements) {
      assert.ok(Number.isFinite(placement.top) && Number.isFinite(placement.height));
    }
  }
});

if (failures > 0) {
  console.error(`\n${failures} 个用例未通过`);
  process.exitCode = 1;
} else {
  console.log("全部通过");
}

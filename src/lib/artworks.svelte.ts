/**
 * 作品列表的分块窗口缓存（Svelte 5 runes）。
 *
 * 元数据廉价、像素昂贵：整库几万条 pid / 路径记录也只有几 MB，而每张缩略图都要
 * 解码一张原图。所以取数是分层的 —— 元数据按「块」按需拉，DOM 由虚拟滚动限制在
 * 可视行，缩略图只在格子进入视口时才生成。列表长度因此不再决定内存与节点规模。
 */

import { api } from "./api";
import type { Artwork } from "./types";

/** 每次向后端要多少条。取大一些可以减少滚动过程中的 IPC 往返。 */
const BLOCK_SIZE = 120;

class ArtworkIndex {
  /** 库里总条数，来自后端的 COUNT(*)。 */
  total = $state(0);
  /** 已经就位的条数。 */
  ready = $state(0);
  /** 是否有块在途中。 */
  busy = $state(false);
  /** 首块是否已经取过，用来区分「还没加载」与「库确实是空的」。 */
  booted = $state(false);
  error = $state<string | null>(null);

  #blocks = new Map<number, Artwork[]>();
  #inflight = new Set<number>();
  /** 重新扫描后自增，用于丢弃上一轮遗留在途的响应。 */
  #generation = 0;
  /**
   * 块数据的响应式信号。
   *
   * 刻意不把块缓存整个丢进 `$state`：Svelte 会给数组里每个对象铺一层代理，
   * 几万条记录光建代理就够呛。虚拟滚动真正需要触发重渲染的只有
   * 「哪些索引已经有数据了」，所以用普通 Map 存数据 + 一个自增计数器通知。
   * `at()` 里读一次 `#revision` 就是在建立这个依赖。
   */
  #revision = $state(0);

  /** 读第 index 条；返回 undefined 表示所在块尚未就位，调用方渲染骨架。 */
  at(index: number): Artwork | undefined {
    void this.#revision;
    const block = Math.floor(index / BLOCK_SIZE);
    return this.#blocks.get(block)?.[index - block * BLOCK_SIZE];
  }

  /** 确保 [start, end) 区间覆盖到的块都在途或已就位。 */
  ensure(start: number, end: number): void {
    if (end <= start) return;
    const first = Math.floor(Math.max(0, start) / BLOCK_SIZE);
    const last = Math.floor(Math.max(0, end - 1) / BLOCK_SIZE);
    for (let block = first; block <= last; block += 1) void this.#fetch(block);
  }

  /** 拉第一块，顺带拿到 total。页面挂载时调用一次。 */
  async bootstrap(): Promise<void> {
    if (this.booted) return;
    await this.#fetch(0);
    this.booted = true;
  }

  /** 重新扫描后清空重来 —— 库里的行变了，旧块全部作废。 */
  reset(): void {
    this.#generation += 1;
    this.#blocks.clear();
    this.#inflight.clear();
    this.#revision += 1;
    this.total = 0;
    this.ready = 0;
    this.busy = false;
    this.booted = false;
    this.error = null;
  }

  async #fetch(block: number): Promise<void> {
    if (this.#blocks.has(block) || this.#inflight.has(block)) return;

    this.#inflight.add(block);
    this.busy = true;
    const generation = this.#generation;

    try {
      const page = await api.listArtworks(block * BLOCK_SIZE, BLOCK_SIZE);
      // 期间重新扫描过 -> 这一块的偏移量已经没有意义，直接丢弃
      if (generation !== this.#generation) return;

      this.#blocks.set(block, page.items);
      this.total = page.total;
      this.ready = [...this.#blocks.values()].reduce(
        (sum, items) => sum + items.length,
        0,
      );
      this.#revision += 1;
    } catch (err) {
      if (generation === this.#generation) this.error = String(err);
    } finally {
      this.#inflight.delete(block);
      this.busy = this.#inflight.size > 0;
    }
  }
}

export const artworks = new ArtworkIndex();

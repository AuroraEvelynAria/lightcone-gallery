/**
 * 作品列表的分块窗口缓存（Svelte 5 runes）。
 *
 * 元数据廉价、像素昂贵：整库几万条 pid / 路径记录也只有几 MB，而每张缩略图都要
 * 解码一张原图。所以取数是分层的 —— 元数据按「块」按需拉，DOM 由虚拟滚动限制在
 * 可视行，缩略图只在格子进入视口时才生成。列表长度因此不再决定内存与节点规模。
 *
 * 瀑布流排版另有一份**整表**的宽高比（见 `aspects()`）：它必须一次拿全，
 * 因为总高 = 最高那一列有多高，缺一条就撑不准滚动条。这份表只在真的切到
 * 瀑布流时才拉，网格模式下一分钱都不花。
 */

import { api } from "./api";
import { RATING_ANY, type Artwork } from "./types";

/** 每次向后端要多少条。取大一些可以减少滚动过程中的 IPC 往返。 */
const BLOCK_SIZE = 120;

/** 排版方式的持久化键。 */
const KEY_LAYOUT = "lightcone.artworks.layout";

/**
 * 排版方式。
 *
 * 没存过时**窄屏默认瀑布流** —— 那正是这套排版存在的理由（手机上一列铺满时
 * 格子大到 1116 物理像素，缩略图被放得发糊；瀑布流保留原图长宽比，所见即所得）。
 * 宽屏保持定高网格：桌面格子小、宽高比差异影响不大，而网格更整齐。
 */
export type ArtworkLayout = "grid" | "masonry";

function readLayout(): ArtworkLayout {
  try {
    const stored = localStorage.getItem(KEY_LAYOUT);
    if (stored === "grid" || stored === "masonry") return stored;
  } catch {
    // 隐私模式下 localStorage 会抛，按「没存过」处理即可
  }
  return window.matchMedia("(max-width: 767px)").matches ? "masonry" : "grid";
}

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
  /** 操作成功的提示（与 error 分开，免得成功也染成红色）。 */
  notice = $state<string | null>(null);

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
  /** 最近一次 `ensure` 的区间。移除之后按它重新取数，用户的滚动位置才不会跳。 */
  #range: [number, number] = [0, 0];

  /** 当前排版方式，持久化到 localStorage。 */
  layout = $state<ArtworkLayout>(readLayout());

  /**
   * 与作品列表**同序**的宽高比（高 / 宽）。只在瀑布流模式下才拉。
   *
   * 不放进 `$state`：四千多个数字铺上代理不值得，理由与块缓存完全一样
   * （见 `#revision`）。响应式靠下面那个自增计数器。
   */
  #aspects: number[] = [];
  #aspectRevision = $state(0);

  /** 比例表是否已就位（条数与列表一致）。瀑布流在它变 true 之前不能排版。 */
  aspectsReady = $state(false);
  aspectsBusy = $state(false);

  /**
   * 当前筛选条件。画师从路径反推、标签来自联网补全的元数据、分级来自
   * `illust_meta.x_restrict`，三者都只影响「取哪些块」，不影响块的切分方式。
   */
  artistUids = $state<string[]>([]);
  pixivTags = $state<string[]>([]);
  /** 分级：见 `types.ts` 的 `RATING_*`。默认不限。 */
  rating = $state<number>(RATING_ANY);

  /**
   * 换筛选条件：作废所有块并从头拉。
   *
   * 与 `refreshAfterRemoval` 不同，这里**必须**重置 `total` —— 筛选后的总数是另一个
   * 数量级，保留旧值会让虚拟网格按错误的行数铺骨架。
   */
  async setFilter(next: {
    artistUids?: string[];
    pixivTags?: string[];
    rating?: number;
  }): Promise<void> {
    this.artistUids = next.artistUids ?? [];
    this.pixivTags = next.pixivTags ?? [];
    this.rating = next.rating ?? RATING_ANY;
    this.reset();
    await this.bootstrap();
  }

  /**
   * 当前条件是不是「什么都没筛」。侧栏靠它决定高亮哪一项、
   * 切换视图时也靠它判断要不要真的重拉。
   */
  get filtered(): boolean {
    return this.artistUids.length > 0 || this.pixivTags.length > 0 || this.rating !== RATING_ANY;
  }

  /**
   * 「请展开筛选面板」的一次性信号，由侧栏的「标签图库 +」发出
   * （新建图库要走「在面板里配好条件 → 存成图库」这条路）。
   *
   * 用**自增计数**而不是布尔量：布尔量置 `true` 之后没法再触发第二次，
   * 而「再新建一条」是很自然的重复动作。
   */
  openFilterRequest = $state(0);

  requestOpenFilter(): void {
    this.openFilterRequest += 1;
  }

  /** 切排版方式并落盘。切到瀑布流时调用方要跟着 `ensureAspects()`。 */
  setLayout(next: ArtworkLayout): void {
    if (next === this.layout) return;
    this.layout = next;
    try {
      localStorage.setItem(KEY_LAYOUT, next);
    } catch {
      // 存不下就只在本次会话生效，不影响功能
    }
  }

  /**
   * 与列表同序的宽高比。读到 `#aspectRevision` 就是在建立响应式依赖 ——
   * 表一变，用到它的 `$derived` 会重算。
   */
  aspects(): number[] {
    void this.#aspectRevision;
    return this.#aspects;
  }

  /**
   * 拉整表的宽高比。瀑布流排版前必须调一次。
   *
   * **只在瀑布流模式下按需拉**：网格模式完全不需要它，白拉一份四千多条的
   * 数据只会拖慢首屏。首次调用后端还要现场量老库的尺寸（读几千个文件头），
   * 所以慢一点是正常的，界面要有「正在准备布局」的提示。
   */
  async ensureAspects(): Promise<void> {
    if (this.aspectsBusy || this.aspectsReady) return;

    this.aspectsBusy = true;
    const generation = this.#generation;

    try {
      const list = await api.listArtworkAspects({
        artistUids: this.artistUids,
        pixivTags: this.pixivTags,
        rating: this.rating,
      });
      // 期间换过筛选 / 重新扫描过 -> 这份表对的是另一批数据，丢掉
      if (generation !== this.#generation) return;

      this.#aspects = list.map((item) => item.aspect);
      this.#aspectRevision += 1;
      this.aspectsReady = this.#aspects.length > 0;
    } catch (err) {
      if (generation === this.#generation) this.error = String(err);
    } finally {
      this.aspectsBusy = false;
    }
  }

  /** 列表内容变了（换筛选 / 移除 / 重扫）之后比例表也不再对得上，必须作废重拉。 */
  #invalidateAspects(): void {
    this.#aspects = [];
    this.#aspectRevision += 1;
    this.aspectsReady = false;
  }

  /** 读第 index 条；返回 undefined 表示所在块尚未就位，调用方渲染骨架。 */
  at(index: number): Artwork | undefined {
    void this.#revision;
    const block = Math.floor(index / BLOCK_SIZE);
    return this.#blocks.get(block)?.[index - block * BLOCK_SIZE];
  }

  /** 确保 [start, end) 区间覆盖到的块都在途或已就位。 */
  ensure(start: number, end: number): void {
    if (end <= start) return;
    this.#range = [start, end];
    const first = Math.floor(Math.max(0, start) / BLOCK_SIZE);
    const last = Math.floor(Math.max(0, end - 1) / BLOCK_SIZE);
    for (let block = first; block <= last; block += 1) void this.#fetch(block);
  }

  /**
   * 移除若干条之后刷新。
   *
   * 不能简单地 `reset()`：那会把 `total` 归零、虚拟网格整个卸载，用户的滚动
   * 位置就丢了 —— 在 2000 多条里移除一张，代价不该是「回到顶部」。
   * 这里只作废块缓存并保留 `total`（后端返回了实际删掉的行数），
   * 再按上次的可视区间重新取数；期间网格留在原地，只是短暂显示骨架。
   */
  refreshAfterRemoval(removed: number): void {
    this.total = Math.max(0, this.total - removed);
    this.ready = 0;
    this.#generation += 1;
    this.#blocks.clear();
    this.#inflight.clear();
    this.#revision += 1;
    // 少了一条，比例表的下标就整体错位了，必须重拉
    this.#invalidateAspects();
    this.error = null;

    if (this.total > 0) this.ensure(this.#range[0], this.#range[1]);
  }

  /** 拉第一块，顺带拿到 total。页面挂载时调用一次。 */
  async bootstrap(): Promise<void> {
    if (this.booted) return;
    await this.#fetch(0);
    this.booted = true;
  }

  /**
   * 取第 index 条，所在块还没到位就等它到。弹窗翻页用。
   *
   * 虚拟滚动是「滚到哪取哪」，翻页会跳到当前窗口之外，所以这里必须能等。
   * 超时返回 undefined，让调用方保持原样 —— 宁可不动，也不要卡在空白弹窗上。
   */
  async fetchAt(index: number): Promise<Artwork | undefined> {
    if (index < 0 || index >= this.total) return undefined;

    const existing = this.at(index);
    if (existing) return existing;

    this.ensure(index, index + 1);
    const deadline = Date.now() + 3000;
    while (Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 30));
      const item = this.at(index);
      if (item) return item;
    }
    return undefined;
  }

  /** 重新扫描后清空重来 —— 库里的行变了，旧块全部作废。 */
  reset(): void {
    this.#generation += 1;
    this.#blocks.clear();
    this.#inflight.clear();
    this.#revision += 1;
    this.#invalidateAspects();
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
      const page = await api.listArtworks(block * BLOCK_SIZE, BLOCK_SIZE, {
        artistUids: this.artistUids,
        pixivTags: this.pixivTags,
        rating: this.rating,
      });
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

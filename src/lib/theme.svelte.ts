/**
 * 外观状态（Svelte 5 runes）：配色方案 × 窗口材质 × 明暗，三个正交的轴。
 *
 * 两条持久化路径，各有分工：
 * - **数据库 `settings` 表是权威来源**。WebView 的 localStorage 会被清缓存顺手清掉，
 *   用户挑的外观不该跟着消失。
 * - **localStorage 只作首帧缓存**。构造时同步读一次并立刻落到 DOM 上，
 *   这样页面第一帧就是用户选的样子，不会先闪一下默认配色再跳。
 */

import { api } from "./api";
import {
  findMaterial,
  findScheme,
  isMaterialId,
  isSchemeId,
  isThemeMode,
  type MaterialId,
  type Palette,
  type SchemeId,
  type ThemeMode,
} from "./themes";

/** settings 表里的键名 */
const KEY_SCHEME = "theme.scheme";
const KEY_MATERIAL = "theme.material";
const KEY_MODE = "theme.mode";

/** localStorage 的键前缀。与下面的 `KEY_*` 拼出 `lightcone.theme.scheme` 这样的键。 */
const LS_PREFIX = "lightcone.";

/**
 * 材质生效时应用底色要让出多少透明度。
 *
 * 系统材质画在窗口最底层，WebView 的内容盖在它上面 —— 底色若是不透明的，
 * 材质就被完全遮住、等于没开。所以透明材质要配一组半透明底色。
 * 三个值分开给：底色让得最多（材质最明显），卡片让得最少（保证图片对比度）。
 */
const ALPHA_TRANSLUCENT = { bg: 0.5, panel: 0.66, case: 0.74 };
const ALPHA_SOLID = { bg: 1, panel: 1, case: 1 };

/** 把一组令牌写成 `--tone-*` 自定义属性。 */
function writePalette(root: HTMLElement, palette: Palette): void {
  for (const [token, value] of Object.entries(palette)) {
    root.style.setProperty(`--tone-${token}`, value);
  }
}

function readCache(key: string): string | null {
  try {
    return localStorage.getItem(LS_PREFIX + key);
  } catch {
    // localStorage 不可用（隐私模式等）就当没缓存
    return null;
  }
}

function writeCache(key: string, value: string): void {
  try {
    localStorage.setItem(LS_PREFIX + key, value);
  } catch {
    // 忽略
  }
}

class ThemeStore {
  scheme = $state<SchemeId>("archive");
  material = $state<MaterialId>("solid");
  mode = $state<ThemeMode>("system");

  /** 系统当前是否偏好深色 */
  #systemDark = $state(true);
  /** 是否已经把数据库里的权威设置读回来过 */
  #synced = false;
  /** 最近一次下发成功的「材质 + 明暗」，用来避免重复 IPC */
  #applied: string | null = null;

  error = $state<string | null>(null);

  /** 解析后的实际明暗 */
  dark = $derived(this.mode === "dark" || (this.mode === "system" && this.#systemDark));
  /** 当前生效的调色板 */
  palette = $derived<Palette>(
    this.dark ? findScheme(this.scheme).dark : findScheme(this.scheme).light,
  );

  constructor() {
    // 首帧缓存：同步读、同步落 DOM，页面第一帧就是用户选的样子
    const scheme = readCache("scheme");
    const material = readCache("material");
    const mode = readCache("mode");
    if (isSchemeId(scheme)) this.scheme = scheme;
    if (isMaterialId(material)) this.material = material;
    if (isThemeMode(mode)) this.mode = mode;

    // 系统偏好也要在首帧就知道，否则 "system" 模式会先按深色渲染再跳
    try {
      this.#systemDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    } catch {
      // matchMedia 不可用就维持深色默认
    }

    this.#writeDom();
  }

  /** 在根布局挂载时调用一次：接上系统偏好监听 + 读回数据库里的权威设置。 */
  async init(): Promise<void> {
    const mql = window.matchMedia("(prefers-color-scheme: dark)");
    mql.addEventListener("change", (event) => {
      this.#systemDark = event.matches;
    });

    try {
      const settings = await api.loadSettings();
      if (isSchemeId(settings[KEY_SCHEME])) this.scheme = settings[KEY_SCHEME];
      if (isMaterialId(settings[KEY_MATERIAL])) this.material = settings[KEY_MATERIAL];
      if (isThemeMode(settings[KEY_MODE])) this.mode = settings[KEY_MODE];
    } catch (err) {
      this.error = String(err);
    }

    this.#synced = true;
    // 数据库里的值可能与首帧缓存不同，重新落一次 DOM
    this.#writeDom();
    void this.#pushMaterial();
  }

  /**
   * 由根布局的 `$effect` 调用：状态一变就把令牌与材质同步到 DOM / 窗口。
   *
   * 函数内部读了 `palette` / `material`，所以在 effect 里调用就能建立依赖；
   * 它只写 DOM 与自定义属性、不写自身状态，不会形成回环。
   */
  apply(): void {
    this.#writeDom();
    void this.#pushMaterial();
  }

  setScheme(scheme: SchemeId): void {
    this.scheme = scheme;
    this.#persist(KEY_SCHEME, scheme);
  }

  setMaterial(material: MaterialId): void {
    this.material = material;
    this.#persist(KEY_MATERIAL, material);
  }

  setMode(mode: ThemeMode): void {
    this.mode = mode;
    this.#persist(KEY_MODE, mode);
  }

  #persist(key: string, value: string): void {
    writeCache(key, value);
    if (!this.#synced) return;

    void api.saveSettings({ [key]: value }).catch((err) => {
      this.error = String(err);
    });
  }

  /** 把令牌、明暗标记、材质透明度写到 `<html>` 上。 */
  #writeDom(): void {
    const root = document.documentElement;
    writePalette(root, this.palette);

    root.dataset.scheme = this.scheme;
    root.dataset.mode = this.dark ? "dark" : "light";

    const alpha = findMaterial(this.material).translucent ? ALPHA_TRANSLUCENT : ALPHA_SOLID;
    root.style.setProperty("--alpha-bg", String(alpha.bg));
    root.style.setProperty("--alpha-panel", String(alpha.panel));
    root.style.setProperty("--alpha-case", String(alpha.case));
  }

  /** 下发窗口材质。材质或明暗没变就不重复调用。 */
  async #pushMaterial(): Promise<void> {
    const key = `${this.material}:${this.dark}`;
    if (key === this.#applied) return;
    this.#applied = key;

    try {
      await api.setWindowMaterial(this.material, this.dark);
      this.error = null;
    } catch (err) {
      this.error = String(err);
    }
  }
}

export const theme = new ThemeStore();

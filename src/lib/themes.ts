/**
 * 配色方案与窗口材质。
 *
 * **配色**源自 RhinE（foobar2000 主题）的 `tokens/tokens.json`
 * —— <https://github.com/ericzhang12111-cell/RhinE>
 * 9 个方案 × 明暗两套 × 15 个语义令牌。它那套词汇（bg / panel / case / well /
 * hair / line-* / accent / on-accent）本来就是给「机箱陈列」这类工业界面设计的，
 * 正好对上我们要的观感，所以直接沿用，没有另起一套名字。
 *
 * 对 RhinE 原样的**两处偏离**（都是 2026-10-11 主人反馈后改的）：
 * - FLARE 的浅色版原样把整面背景都做成了橙底（警示牌式的设计），当图库背景
 *   没法用 —— 改成暖纸底，橙色列回强调位；深色版本就很好，保持原样。
 * - 追加了 LAGOON / IRIS / BLOSSOM / CRIMSON 四套，补上青、紫、粉、红四个
 *   色相。令牌结构照抄 ARCHIVE / COLD FRONT 的明暗两版套路。
 *
 * **材质**是 Tauri 的真实系统效果（DWM Mica / Acrylic / Tabbed），
 * 不是 CSS 假模糊。窗口必须 `transparent: true` 且 `html, body` 背景透明，
 * 否则材质被自己的底色盖住看不见。
 */

export const SCHEME_IDS = [
  'archive',
  'hazard',
  'flare',
  'field',
  'front',
  'lagoon',
  'iris',
  'blossom',
  'crimson',
] as const;
export type SchemeId = (typeof SCHEME_IDS)[number];

/** 一个方案在某种明暗下的 15 个语义令牌。 */
export interface Palette {
  /** 窗口底色 */
  bg: string;
  /** 正文前景 */
  fg: string;
  /** 次级前景 */
  "fg-soft": string;
  /** 弱化文本（说明、计数） */
  "text-muted": string;
  /** 最弱的标记色（分隔符、序号） */
  mark: string;
  /** 强分隔线 */
  "line-dim": string;
  /** 弱分隔线 */
  "line-faint": string;
  /** 1px 发丝线 */
  hair: string;
  /** 面板底 */
  panel: string;
  /** 面板高光面 */
  "panel-hi": string;
  /** 卡片底（网格里的格子） */
  case: string;
  /** 卡片高光面 */
  "case-hi": string;
  /** 内凹区域（输入框、凹槽） */
  well: string;
  /** 强调色 */
  accent: string;
  /** 强调色上的文字 */
  "on-accent": string;
}

export interface Scheme {
  id: SchemeId;
  /** 界面显示名，沿用 RhinE 的命名 */
  name: string;
  light: Palette;
  dark: Palette;
}

export const SCHEMES: readonly Scheme[] = [
  {
    id: "archive",
    name: "ARCHIVE",
    light: {
      bg: "#eeede7",
      fg: "#252821",
      "fg-soft": "#4d4f49",
      "text-muted": "#63665d",
      mark: "#8d9185",
      "line-dim": "#8a8a84",
      "line-faint": "#cacac3",
      hair: "#d2d1cb",
      panel: "#f4f3ee",
      "panel-hi": "#faf9f4",
      case: "#e4e2da",
      "case-hi": "#fbfaf5",
      well: "#e3e2db",
      accent: "#ff7a01",
      "on-accent": "#0b0b0b",
    },
    dark: {
      bg: "#0b0b0b",
      fg: "#f2f0ea",
      "fg-soft": "#bbb9b4",
      "text-muted": "#8c8b88",
      mark: "#8a8984",
      "line-dim": "#73726f",
      "line-faint": "#393938",
      hair: "#30302f",
      panel: "#111110",
      "panel-hi": "#181817",
      case: "#1b1b1a",
      "case-hi": "#2b2b29",
      well: "#121212",
      accent: "#ff7a01",
      "on-accent": "#0b0b0b",
    },
  },
  {
    id: "hazard",
    name: "HAZARD",
    light: {
      bg: "#ebe8dc",
      fg: "#1a1a16",
      "fg-soft": "#45443c",
      "text-muted": "#5f5d53",
      mark: "#86857d",
      "line-dim": "#8a877c",
      "line-faint": "#c5c3b8",
      hair: "#cecbc0",
      panel: "#f2efe2",
      "panel-hi": "#f8f5e8",
      case: "#dfdcd1",
      "case-hi": "#f9f6e9",
      well: "#dfdcd1",
      accent: "#f2b800",
      "on-accent": "#1a1a16",
    },
    dark: {
      bg: "#0d0d0b",
      fg: "#f2efe4",
      "fg-soft": "#bcb8ab",
      "text-muted": "#8f8b7f",
      mark: "#8a8982",
      "line-dim": "#6f6c62",
      "line-faint": "#3b3a37",
      hair: "#32312e",
      panel: "#131310",
      "panel-hi": "#1a1a17",
      case: "#1d1d1a",
      "case-hi": "#2d2c29",
      well: "#141412",
      accent: "#ffcf1a",
      "on-accent": "#0d0d0b",
    },
  },
  {
    id: "flare",
    name: "FLARE",
    light: {
      // 原样是整面橙底（RhinE 的警示牌设计），当图库背景没法用 —— 改成暖纸底，
      // 让 FLARE 橙回到强调位。深色版没动。
      bg: "#f5ece2",
      fg: "#26170c",
      "fg-soft": "#503a28",
      "text-muted": "#6a594a",
      mark: "#97897b",
      "line-dim": "#94887a",
      "line-faint": "#d3c6b7",
      hair: "#ddd1c2",
      panel: "#fbf3e9",
      "panel-hi": "#fff9f0",
      case: "#ecdfd0",
      "case-hi": "#fff8ec",
      well: "#ecdfd0",
      accent: "#ff6b1a",
      "on-accent": "#1c0d02",
    },
    dark: {
      bg: "#140a05",
      fg: "#ffe9da",
      "fg-soft": "#ffb991",
      "text-muted": "#d98a5c",
      mark: "#95847a",
      "line-dim": "#9a5a35",
      "line-faint": "#433730",
      hair: "#3a2e27",
      panel: "#1a100a",
      "panel-hi": "#211611",
      case: "#241914",
      "case-hi": "#342922",
      well: "#1b110c",
      accent: "#ff6b1a",
      "on-accent": "#140a05",
    },
  },
  {
    id: "field",
    name: "FIELD",
    light: {
      bg: "#edf1e6",
      fg: "#1c261a",
      "fg-soft": "#435040",
      "text-muted": "#59665a",
      mark: "#888f84",
      "line-dim": "#859183",
      "line-faint": "#c7cdc1",
      hair: "#d0d4c9",
      panel: "#f4f7ed",
      "panel-hi": "#fafef3",
      case: "#e1e5da",
      "case-hi": "#fbfff4",
      well: "#e1e5da",
      accent: "#1fa84f",
      "on-accent": "#1c261a",
    },
    dark: {
      bg: "#090d09",
      fg: "#e7f1e3",
      "fg-soft": "#b2c0ad",
      "text-muted": "#869683",
      mark: "#838a80",
      "line-dim": "#66755f",
      "line-faint": "#363b35",
      hair: "#2d322c",
      panel: "#0f130e",
      "panel-hi": "#151a15",
      case: "#181d18",
      "case-hi": "#282c27",
      well: "#101410",
      accent: "#4fdc76",
      "on-accent": "#090d09",
    },
  },
  {
    id: "front",
    name: "COLD FRONT",
    light: {
      bg: "#e9eef3",
      fg: "#15202b",
      "fg-soft": "#3e4a57",
      "text-muted": "#56636f",
      mark: "#838b93",
      "line-dim": "#84909b",
      "line-faint": "#c3c9cf",
      hair: "#cbd1d7",
      panel: "#f0f5f9",
      "panel-hi": "#f6fbff",
      case: "#dde2e8",
      "case-hi": "#f7fcff",
      well: "#dde2e8",
      accent: "#1f6fff",
      "on-accent": "#e9eef3",
    },
    dark: {
      bg: "#070a0f",
      fg: "#e5edf6",
      "fg-soft": "#aebccb",
      "text-muted": "#83919f",
      mark: "#81868d",
      "line-dim": "#5f6d7b",
      "line-faint": "#34383d",
      hair: "#2b2f34",
      panel: "#0d1015",
      "panel-hi": "#13171c",
      case: "#161a1f",
      "case-hi": "#26292f",
      well: "#0e1116",
      accent: "#3d8bff",
      "on-accent": "#070a0f",
    },
  },
  {
    id: "lagoon",
    name: "LAGOON",
    light: {
      bg: "#e6efed",
      fg: "#0f2523",
      "fg-soft": "#374f4c",
      "text-muted": "#4f6663",
      mark: "#7e918e",
      "line-dim": "#7d8f8c",
      "line-faint": "#c0cfcc",
      hair: "#cbd8d5",
      panel: "#edf5f3",
      "panel-hi": "#f2fbf9",
      case: "#d9e5e2",
      "case-hi": "#effaf8",
      well: "#d9e5e2",
      accent: "#0e9188",
      "on-accent": "#eaf6f4",
    },
    dark: {
      bg: "#060c0b",
      fg: "#e0efec",
      "fg-soft": "#a3bcb8",
      "text-muted": "#7b918d",
      mark: "#7d8683",
      "line-dim": "#5a6e6a",
      "line-faint": "#313b39",
      hair: "#2a3331",
      panel: "#0a1211",
      "panel-hi": "#101a18",
      case: "#121d1b",
      "case-hi": "#22302d",
      well: "#0b1312",
      accent: "#2ed3c3",
      "on-accent": "#06110f",
    },
  },
  {
    id: "iris",
    name: "IRIS",
    light: {
      bg: "#ece9f3",
      fg: "#1d1730",
      "fg-soft": "#443c5e",
      "text-muted": "#5c5474",
      mark: "#8a84a0",
      "line-dim": "#867f9c",
      "line-faint": "#cbc5da",
      hair: "#d3cee0",
      panel: "#f3f0f9",
      "panel-hi": "#f9f6fe",
      case: "#e1dcea",
      "case-hi": "#f8f4ff",
      well: "#e1dcea",
      accent: "#7c3aed",
      "on-accent": "#f3f0f9",
    },
    dark: {
      bg: "#0a0813",
      fg: "#eae6f6",
      "fg-soft": "#b5aecb",
      "text-muted": "#8a83a1",
      mark: "#858190",
      "line-dim": "#645d7e",
      "line-faint": "#363249",
      hair: "#2f2b40",
      panel: "#0f0d1a",
      "panel-hi": "#161327",
      case: "#151225",
      "case-hi": "#272242",
      well: "#0d0b16",
      accent: "#a78bfa",
      "on-accent": "#0a0813",
    },
  },
  {
    id: "blossom",
    name: "BLOSSOM",
    light: {
      bg: "#f4eaee",
      fg: "#2c151f",
      "fg-soft": "#563a47",
      "text-muted": "#6f545f",
      mark: "#9d8791",
      "line-dim": "#99818c",
      "line-faint": "#d9c7cf",
      hair: "#e1d1d8",
      panel: "#faf0f4",
      "panel-hi": "#fff5f9",
      case: "#ecdbe2",
      "case-hi": "#fff3f8",
      well: "#ecdbe2",
      accent: "#d6336c",
      "on-accent": "#fdf2f6",
    },
    dark: {
      bg: "#120a0e",
      fg: "#f5e7ed",
      "fg-soft": "#c5a9b5",
      "text-muted": "#947f8a",
      mark: "#8d8087",
      "line-dim": "#745e69",
      "line-faint": "#3e3239",
      hair: "#362b31",
      panel: "#180f13",
      "panel-hi": "#201519",
      case: "#1f1218",
      "case-hi": "#312329",
      well: "#140c10",
      accent: "#f0679e",
      "on-accent": "#120a0e",
    },
  },
  {
    id: "crimson",
    name: "CRIMSON",
    light: {
      bg: "#f2eaea",
      fg: "#2b1416",
      "fg-soft": "#583a3c",
      "text-muted": "#715759",
      mark: "#9d888a",
      "line-dim": "#988284",
      "line-faint": "#dccacb",
      hair: "#e3d2d3",
      panel: "#f9f0f0",
      "panel-hi": "#fff5f5",
      case: "#eadbdc",
      "case-hi": "#fff3f4",
      well: "#eadbdc",
      accent: "#d22638",
      "on-accent": "#fdf2f3",
    },
    dark: {
      bg: "#100809",
      fg: "#f5e6e7",
      "fg-soft": "#c7a9ab",
      "text-muted": "#968082",
      mark: "#8e8183",
      "line-dim": "#776062",
      "line-faint": "#403234",
      hair: "#382c2e",
      panel: "#170d0e",
      "panel-hi": "#1f1314",
      case: "#1e1213",
      "case-hi": "#2f2223",
      well: "#120a0b",
      accent: "#ef4655",
      "on-accent": "#100809",
    },
  },
];

export function findScheme(id: SchemeId): Scheme {
  return SCHEMES.find((scheme) => scheme.id === id) ?? SCHEMES[0];
}

// ---------------------------------------------------------------------------
// 窗口材质
// ---------------------------------------------------------------------------

export const MATERIAL_IDS = ['solid', 'mica', 'acrylic', 'tabbed', 'blur'] as const;
export type MaterialId = (typeof MATERIAL_IDS)[number];

export interface Material {
  id: MaterialId;
  name: string;
  /** 选项下方的说明 */
  note: string;
  /**
   * 是否透出桌面/壁纸。true 时应用底色与面板会按 `--alpha-*` 半透明，
   * 否则系统材质被自己的底色完全盖住、等于没开。
   */
  translucent: boolean;
  /** 平台要求，直接显示给用户 */
  platform: string;
}

export const MATERIALS: readonly Material[] = [
  {
    id: 'solid',
    name: '纯色',
    note: '不启用系统材质，只用配色方案自身的底色',
    translucent: false,
    platform: '全平台',
  },
  {
    id: 'mica',
    name: '云母',
    note: '桌面壁纸的柔和采样，随窗口拖动轻微流动',
    translucent: true,
    platform: 'Windows 11',
  },
  {
    id: 'acrylic',
    name: '亚克力',
    note: '比云母更透、带高斯模糊，观感更「玻璃」',
    translucent: true,
    platform: 'Windows 10 v1809+',
  },
  {
    id: 'tabbed',
    name: '标签页',
    note: '比云母更暗更实，适合长时间盯着看',
    translucent: true,
    platform: 'Windows 11',
  },
  {
    id: 'blur',
    name: '模糊',
    note: '旧版高斯模糊。Windows 11 22621 上拖动窗口会卡',
    translucent: true,
    platform: 'Windows 7 / 10 / 11 22H1',
  },
];

export function findMaterial(id: MaterialId): Material {
  return MATERIALS.find((material) => material.id === id) ?? MATERIALS[0];
}

export type ThemeMode = 'system' | 'light' | 'dark';

// ---------------------------------------------------------------------------
// 取值校验
//
// 设置是从磁盘读回来的，不能假定它还有效 —— 比如以后删掉某个方案，
// 旧设置里还留着它的 id，直接塞进状态会让界面落到一个不存在的配色上。
// ---------------------------------------------------------------------------

export function isSchemeId(value: unknown): value is SchemeId {
  return typeof value === 'string' && (SCHEME_IDS as readonly string[]).includes(value);
}

export function isMaterialId(value: unknown): value is MaterialId {
  return typeof value === 'string' && (MATERIAL_IDS as readonly string[]).includes(value);
}

export function isThemeMode(value: unknown): value is ThemeMode {
  return value === 'system' || value === 'light' || value === 'dark';
}

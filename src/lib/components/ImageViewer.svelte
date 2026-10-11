<script module lang="ts">
  import { api } from "$lib/api";

  /**
   * 光锥视差特效的总开关，**两个弹窗共用一份**（放在 module 里）。
   *
   * 关掉之后弹窗变成朴素的平面显示：不倾斜、不扫光、不发光、不入场动画。
   * 状态存在 `settings.viewer_effects` 里，重启后还在。
   */
  let effectsEnabled = $state(true);
  let effectsLoaded = false;

  /** 只读一次设置，之后靠内存里的值。 */
  export async function ensureEffectsLoaded(): Promise<void> {
    if (effectsLoaded) return;
    effectsLoaded = true;
    try {
      const settings = await api.loadSettings();
      if (settings["viewer_effects"] === "0") effectsEnabled = false;
    } catch {
      // 读不到就用默认值（开着），不该因为一个偏好读失败而影响看图
    }
  }

  export function toggleEffects(): void {
    effectsEnabled = !effectsEnabled;
    void api
      .saveSettings({ viewer_effects: effectsEnabled ? "1" : "0" })
      .catch(() => {
        // 存不下不影响这次会话，下次启动回到默认值
      });
  }

  export function effectsOn(): boolean {
    return effectsEnabled;
  }
</script>

<script lang="ts">
  /**
   * 图片查看器内核：三层嵌套光锥视差 + 缩放平移 + 全屏。纯 CSS 3D，无 WebGL。
   *
   * 通用图库和作品库两个弹窗共用它。之前这套视差只长在作品库那边，而主人真正
   * 在浏览的是通用图库 —— 抽成一层之后两边是同一套手感。
   *
   * ## 层结构
   *
   * ```
   * .viewer                固定全屏 + 背幕
   *   .viewer__stage       perspective: 900px（只此一处）
   *     .viewer__frame     量「去掉留白的整块」；放大时框最多撑到这里
   *       .viewer__inner   量「带留白的区域」；1× 的适应尺寸按它算
   *         .viewer__stack 竖排：卡片在上，信息行在下
   *           .viewer__scene   入场动画（缩放 + 轻微翻转）
   *             .viewer__card  preserve-3d，跟鼠标做 rotateX / rotateY
   *               .viewer__ambient translateZ(-64px) 环境光（同一张原图糊开）
   *               .viewer__halo   translateZ(-96px)  光锥底（accent 背光场）
   *               .viewer__shot   translateZ(0)      原图（圆角裁切在这层）
   *               .viewer__glare  translateZ(24px)   扫光（overlay）
   *               .viewer__sheen  translateZ(26px)   斜光（color-dodge）
   *           .viewer__meta    普通布局，**不在 3D 层里**（见下）；只吃
   *                            --meta-x/y 的 ±6px 平移微视差，不旋转
   * ```
   *
   * ## 两条必须守住的规则
   *
   * 1. `overflow != visible`、`opacity < 1`、`filter`、`mix-blend-mode != normal`、
   *    `contain: paint` 都会**把该元素的 `transform-style` 强制降级为 flat**
   *    （CSS Transforms L2 的「分组属性」）。所以圆角裁切只能放在最内层的图片上，
   *    而 `mix-blend-mode` 的扫光必须是**叶子**。
   * 2. 缩放不能加在 `.viewer__shot` 上：那层带 `overflow: hidden`，`scale()` 会把
   *    圆角一起放大。缩放加在它里面的 `<img>` 上，裁切框保持不动 —— 观感上就是
   *    「透过一张卡片看放大的图」。
   *
   * ## 尺寸为什么用 JS 算
   *
   * 图片的显示尺寸由 `natural` × 容器盒子在 JS 里算成整数像素，写死在
   * `.viewer__shot` 上。不用 `max-height: 100%` 那套是因为百分比 `max-height`
   * 在「高度 auto 的祖先链」里会被当成 `none` 直接失效，而这个组件必须知道
   * 图片的精确显示尺寸才能把平移范围夹在边缘内。
   *
   * ## 参考
   *
   * 层距与阻尼取值参考 `simeydotme/pokemon-cards-css`（同为 Svelte 项目，
   * 纯 CSS 做鼠标跟随全息高光）。RGB 偏移来自 `abx15/3d-interactive-gallery`，
   * 那边是 WebGL shader，这里用双层 `drop-shadow` 做等效的入场脉冲。
   */
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { untrack } from "svelte";
  import type { Snippet } from "svelte";

  interface Props {
    /** 要显示的图片地址（已 convertFileSrc 过的原图） */
    src: string;
    alt: string;
    /** 前景信息层，浮在图片下沿（跟着 3D 一起抬起来） */
    meta?: Snippet;
    /** 底部工具条右侧的附加控件 */
    actions?: Snippet;
    /** Esc / 点空白 / 关闭按钮 */
    onclose: () => void;
    /** 上一张 / 下一张。不传就不显示箭头，也不响应左右方向键 */
    onnavigate?: (delta: number) => void;
    /** 全屏状态变化。父组件据此收起侧边信息面板 */
    onfullscreen?: (value: boolean) => void;
    /** 数字键 1-5 直接评分。不传就不响应数字键 */
    onrate?: (rating: number) => void;
    /** 加载失败时展示的路径，方便主人自己去磁盘上找 */
    failurePath?: string;
    /** 原图在磁盘上的路径。给了就在工具条上多一个「复制图片」按钮 */
    copyPath?: string;
  }

  let {
    src,
    alt,
    meta,
    actions,
    onclose,
    onnavigate,
    onfullscreen,
    onrate,
    failurePath,
    copyPath,
  }: Props = $props();

  /** 最大倾角。卡牌类效果常用 ±14°，检视大图收敛到 7° 更稳。 */
  const MAX_TILT = 7;
  /**
   * 粗指针设备（重力感应）的倾角上限。手机上没有指针的精确移动，
   * 倾斜本身就是输入——用足卡牌标准的 ±14°，比桌面鼠标的 7° 更放得开。
   */
  const GYRO_MAX_TILT = 14;
  /** 信息行微视差幅度（px）。沿鼠标反方向浮，要空间感但不动摇可读性。 */
  const META_PARALLAX = 6;
  /**
   * 重力感应的行程：偏离中性握持角 ±RANGE 度即满倾斜。
   * 首次读到传感器数据时把当时的姿态记为中性——人不会把手机举得端端正正，
   * 自动校准比要求「握正」现实；回到前台时重新校准（握持姿势多半变了）。
   */
  const GYRO_RANGE = 15;
  /** 每帧向目标插值的比例，≈140ms 时间常数。 */
  const EASE = 0.12;
  /** 收敛阈值，低于它就停掉 rAF。 */
  const SETTLE = 0.02;
  /** 缩放的插值比例。比倾斜大得多 —— 滚轮要「跟手」，只是逐帧合并后写一次。 */
  const ZOOM_EASE = 0.4;
  /** 缩放的收敛阈值，单位是像素，所以要比角度那个小得多。 */
  const ZOOM_SETTLE = 0.05;
  /** 放大上限。再往上就是马赛克了，没有意义。 */
  const MAX_ZOOM = 8;
  /** 滚轮灵敏度：deltaY 每 1px 乘 exp(0.001)，一格标准滚轮约 ×1.11。 */
  const WHEEL_SENSITIVITY = 0.001;
  /**
   * 从「适应窗口」起步的第一格倍率。
   *
   * 一滚轮通常是「想放大看」，所以第一格多走一点（到 130%），之后每格才是细步。
   * 主人反馈过「轻轻一推滚轮就是 207%，太灵敏」—— 那 207% 来自「一放大就铺满」
   * 那套（第一格会额外乘上 coverScale）。现在铺满交给双击，滚轮只负责细腻。
   */
  const FIRST_STEP = 1.3;
  /** 拖拽超过这个距离就不算「点击」，避免拖完顺手把弹窗关了。 */
  const DRAG_SLOP = 5;

  let stage = $state<HTMLElement | null>(null);
  /** 外层：量「去掉四周留白」的整块可用区域 */
  let frame = $state<HTMLElement | null>(null);
  /** 内层：量「带留白」的区域，1× 时图片按它算适应尺寸 */
  let inner = $state<HTMLElement | null>(null);
  let shot = $state<HTMLElement | null>(null);

  let loaded = $state(false);
  let failed = $state(false);
  let fullscreen = $state(false);

  /** 原图像素尺寸，`onload` 时才拿得到。 */
  let natural = $state({ w: 0, h: 0 });
  /** 带留白的可用区域。1× 的适应尺寸按它算。 */
  let box = $state({ w: 0, h: 0 });
  /** 去掉留白的整块区域。放大时显示框最多撑到这里 —— 这就是「去掉四周留白」。 */
  let full = $state({ w: 0, h: 0 });

  /**
   * 信息行占的高度。
   *
   * CSS 里给它固定高度 + 单行省略，这里才敢当常量用 —— 否则「图片 + 信息行」
   * 会比可用区域高，信息行被挤出画面外。
   */
  const CAPTION_H = 30;
  /** 图片与信息行之间的间距（`.viewer__stack` 的 gap，0.5rem）。 */
  const STACK_GAP = 8;
  /**
   * `.viewer__inner` 在 100% 时的上内边距（CSS 里的 1rem）。
   *
   * 只用来算「100% ↔ 放大」那一步显示框中心的纵向位移（差 `PAD_TOP / 2`，见
   * `centerShiftY`）。放大态的该内边距是 0，所以这里取的是 100% 那一侧的值。
   */
  const PAD_TOP = 16;
  let captionH = $derived(meta ? CAPTION_H : 0);
  /** 底部留白。它**不是**可以吃掉的「留白」—— 工具条就住在那儿。 */
  let padBottom = 0;

  /** 缩放的显示尺寸。**只缩不放**：小图保持原尺寸，免得糊。 */
  let fit = $derived.by(() => {
    const availH = box.h - captionH;
    if (!natural.w || !natural.h || !box.w || availH <= 0) return { w: 0, h: 0 };
    const k = Math.min(box.w / natural.w, availH / natural.h, 1);
    return { w: Math.round(natural.w * k), h: Math.round(natural.h * k) };
  });

  /**
   * 当前渲染值。**刻意不是 `$state`**。
   *
   * 视差和缩放都是高频事件驱动的：滚轮一秒能来上百个事件，鼠标移动同理。
   * 如果每个事件都写 `$state`，Svelte 就会为每个事件重建一次 `style` 属性、
   * 触发一次样式重算 —— 观感就是「缩放不平滑」。所以这些值只是普通变量，
   * 由 rAF 统一插值后**直接写进 DOM**，一帧最多写一次。
   *
   * 只有下面 `zoomPercent` / `zoomed` 是响应式的，因为它们要驱动界面文案与
   * 按钮禁用态，而且只在整数百分比变化时才写。
   */
  let rx = 0;
  let ry = 0;
  let gx = 50;
  let gy = 50;
  let scale = 1;
  let tx = 0;
  let ty = 0;

  // 目标值：事件处理器只改这里，由 rAF 统一插值（高频事件里绝不直接写样式）
  let target = { rx: 0, ry: 0, gx: 50, gy: 50, scale: 1, tx: 0, ty: 0 };
  let frameId = 0;
  /** 上一次写进 DOM 的框尺寸，用来跳过没必要的布局。 */
  let lastFrame = { w: 0, h: 0 };

  /** 显示用的倍率，只在整数变化时才写，避免每帧重渲染。 */
  let zoomPercent = $state(100);
  let zoomed = $state(false);

  let dragging = $state(false);
  /**
   * 「复制图片」的反馈状态。
   *
   * 主人反馈过「右键菜单里有复制图片但点了没反应」—— 静默失败正是问题所在，
   * 所以这里成功失败都要看得见：成功打勾，失败变红并把原因放进 tooltip。
   */
  let copyState = $state<"idle" | "copying" | "done" | "failed">("idle");
  let copyError = $state("");
  let dragStart = { x: 0, y: 0, tx: 0, ty: 0 };
  let dragged = 0;
  /**
   * 这次按下落在「空白」/「图片」上，给 `click` / `dblclick` 判断用。
   *
   * 刻意不是 `$state`：只在事件处理器之间传值，不驱动渲染。为什么不能用事件自己
   * 的 `target`，见 `isBlank`。
   */
  let pressedOnBlank = false;
  let pressedOnPhoto = false;

  /** 系统是否要求减少动效。 */
  const calm = $derived(
    typeof window !== "undefined" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  );

  /**
   * 粗指针设备（手机 / 平板）：没有 hover，光锥的驱动源从鼠标换成重力感应。
   * 触摸拖拽不再参与倾斜（会和陀螺仪打架），只保留放大后的单指平移。
   */
  const touchPrimary = $derived(
    typeof window !== "undefined" &&
      window.matchMedia("(hover: none) and (pointer: coarse)").matches,
  );

  // 重力感应的中性角（非响应式：只在传感器事件里读写）。
  // roll = 左右倾（右倾为正），pitch = 前后倾（顶边后仰为正）——
  // 两种传感器管线都归一化到这两个角，中性角自动校准。
  let neutralRoll = 0;
  let neutralPitch = 0;
  let gyroCalibrated = false;

  /**
   * Generic Sensor 的 GravitySensor（标准 DOM lib 没有它的类型——就地声明
   * 最小形状）。注意「构造器存在」不等于「真的出货」：部分 WebView 有接口
   * 却永远不来 reading，管线探测见下方 effect。重力矢量在设备坐标系里：
   * 手机正立面向用户时 ≈ (0, −9.8, 0)。
   */
  type GravitySensorLike = {
    gravity?: { x: number | null; y: number | null; z: number | null } | null;
    addEventListener(type: "reading" | "error", listener: () => void): void;
    start(): void;
    stop(): void;
  };

  // ---------------------------------------------------------------------------
  // 光锥视差
  // ---------------------------------------------------------------------------

  function onpointermove(event: PointerEvent): void {
    if (dragging) {
      const dx = event.clientX - dragStart.x;
      const dy = event.clientY - dragStart.y;
      dragged = Math.max(dragged, Math.abs(dx) + Math.abs(dy));
      pan(dragStart.tx + dx, dragStart.ty + dy);
      return;
    }

    // 放大之后指针负责平移；粗指针设备的倾斜归重力感应管（两者不打架）
    const node = frame;
    if (!node || calm || zoomed || !effectsEnabled || touchPrimary) return;

    const rect = node.getBoundingClientRect();
    const px = ((event.clientX - rect.left) / rect.width) * 100;
    const py = ((event.clientY - rect.top) / rect.height) * 100;
    // 鼠标在哪一侧，哪一侧就朝自己翻过来（像用手指按住卡片的那一角）
    // 只改倾斜那几项，缩放的 target 不动
    target.ry = -((px - 50) / 50) * MAX_TILT;
    target.rx = ((py - 50) / 50) * MAX_TILT;
    target.gx = px;
    target.gy = py;
    start();
  }

  function resetTilt(): void {
    target.rx = 0;
    target.ry = 0;
    target.gx = 50;
    target.gy = 50;
    start();
  }

  /**
   * 指针离场归位。触屏上跳过：手指抬起就是 pointerleave，而陀螺仪马上会
   * 接管姿态——这里的归位只会和它打架；桌面鼠标离场才需要归位。
   */
  function onpointerleave(event: PointerEvent): void {
    if (touchPrimary) return;
    if (!dragging) resetTilt();
  }

  function start(): void {
    if (!frameId) frameId = requestAnimationFrame(tick);
  }

  /** 把当前值直接写进 DOM。一帧只调一次，绕开 Svelte 的渲染管线。 */
  function paint(): void {
    const stageNode = stage;
    if (stageNode) {
      stageNode.style.setProperty("--rx", `${rx}deg`);
      stageNode.style.setProperty("--ry", `${ry}deg`);
      stageNode.style.setProperty("--gx", `${gx}%`);
      stageNode.style.setProperty("--gy", `${gy}%`);
      // 信息行微视差：直接从插值后的指针位置反推（gx/gy ∈ 0~100），
      // 沿鼠标反方向浮——不新增插值状态，跟扫光共用同一套收敛
      stageNode.style.setProperty("--meta-x", `${-((gx - 50) / 50) * META_PARALLAX}px`);
      stageNode.style.setProperty("--meta-y", `${-((gy - 50) / 50) * META_PARALLAX}px`);
    }
    const shotNode = shot;
    if (shotNode) {
      shotNode.style.setProperty("--s", String(scale));
      shotNode.style.setProperty("--tx", `${tx}px`);
      shotNode.style.setProperty("--ty", `${ty}px`);
      // 框尺寸写 width/height 而不是 scale()：scale 会把圆角一起放大。
      // 只在整数像素变化时才写，避免每帧都触发布局。
      const size = frameSize(scale);
      const w = Math.round(size.w);
      const h = Math.round(size.h);
      if (w !== lastFrame.w || h !== lastFrame.h) {
        lastFrame = { w, h };
        shotNode.style.width = `${w}px`;
        shotNode.style.height = `${h}px`;
      }
    }
  }

  function tick(): void {
    rx += (target.rx - rx) * EASE;
    ry += (target.ry - ry) * EASE;
    gx += (target.gx - gx) * EASE;
    gy += (target.gy - gy) * EASE;
    // 缩放比倾斜跟得紧得多 —— 滚轮要「跟手」，但仍然是逐帧合并后写一次
    scale += (target.scale - scale) * ZOOM_EASE;
    tx += (target.tx - tx) * ZOOM_EASE;
    ty += (target.ty - ty) * ZOOM_EASE;

    const settled =
      Math.abs(target.rx - rx) < SETTLE &&
      Math.abs(target.ry - ry) < SETTLE &&
      Math.abs(target.gx - gx) < SETTLE &&
      Math.abs(target.gy - gy) < SETTLE &&
      Math.abs(target.scale - scale) < ZOOM_SETTLE &&
      Math.abs(target.tx - tx) < ZOOM_SETTLE &&
      Math.abs(target.ty - ty) < ZOOM_SETTLE;

    if (settled) {
      // 收敛时把值钉死在目标上，免得留下 0.001 的偏差
      scale = target.scale;
      tx = target.tx;
      ty = target.ty;
    }

    paint();

    // 显示状态只在整数百分比 / 是否放大变化时才写，避免每帧重渲染
    const percent = Math.round(scale * 100);
    if (percent !== zoomPercent) zoomPercent = percent;
    const isZoomed = target.scale > 1.001 || scale > 1.001;
    if (isZoomed !== zoomed) zoomed = isZoomed;

    // 收敛后停掉，不空转
    frameId = settled ? 0 : requestAnimationFrame(tick);
  }

  // ---------------------------------------------------------------------------
  // 缩放与平移
  // ---------------------------------------------------------------------------

  function clamp(value: number, low: number, high: number): number {
    return value < low ? low : value > high ? high : value;
  }

  /**
   * 显示框最多能长到多大。
   *
   * - 横向：`full.w` —— 左右留白确实可以吃掉。
   * - 纵向：`full.h - 底部留白 - 信息行 - 间距`。底部那截不是留白，工具条住在那儿；
   *   信息行也在图片下面，两者都要占高度。漏扣的后果很直观：框长过头，
   *   把信息行挤出画面外（真的踩过）。
   */
  function maxFrame(): { w: number; h: number } {
    return {
      w: full.w,
      h: Math.max(0, full.h - padBottom - captionH - STACK_GAP),
    };
  }

  /**
   * 显示框（裁切框）在某个倍率下的大小。
   *
   * **框跟着图片一起长**：`fit * scale`，长到「去掉留白的整块」为止，再往上就在
   * 框内裁切。这样图片永远正好填满框 —— 框里绝不会露背景；四周那圈留白是卡片
   * 自己的边距，会随着放大被一格格吃掉。
   *
   * 之前这里是**硬切**的（100% 贴着图，一放大直接占满整块）：主人当时嫌「放大到
   * 一半四周还露着背景」，可硬切又让第一格滚轮必然跳到 `cover` 那么远（实测
   * 207%），反倒嫌「太灵敏」。现在框连续长，两头就都顺了。
   */
  function frameSize(value: number): { w: number; h: number } {
    const cap = maxFrame();
    return {
      w: Math.min(fit.w * value, cap.w),
      h: Math.min(fit.h * value, cap.h),
    };
  }

  /**
   * 「正好铺满」的倍率：图片刚好盖住整块区域，一点不裁。
   *
   * 竖向图、横向图、方图各不相同（实测 137% ~ 200%），所以它是个算出来的值，
   * 不是常量。双击就落到这里 —— 滚轮一格格爬上去的终点也是这里。
   */
  function coverScale(): number {
    if (!fit.w || !fit.h) return 1;
    const cap = maxFrame();
    if (cap.w <= 0 || cap.h <= 0) return 1;
    return Math.max(cap.w / fit.w, cap.h / fit.h, 1);
  }

  /**
   * 一次性写入缩放与位移，并把位移夹在「图片边缘不越过裁切框」的范围内。
   *
   * 图片在倍率 s 下比裁切框大出的部分是 `fit * (s - 1)`，两边各分一半，
   * 所以单侧的最大位移就是 `fit * (s - 1) / 2`。
   */
  function apply(nextScale: number, nextTx: number, nextTy: number): void {
    const next = clamp(nextScale, 1, MAX_ZOOM);
    // 跨过 100% 这条线时把倾斜归零。放大后鼠标改去负责平移（见 `onpointermove`），
    // 倾斜不再更新 —— 不归零的话，放大那一刻歪多少度就永久冻住多少度：画面成
    // 梯形（上下边被压缩），四角还露出背幕。回到 100% 那一步同理。
    if (next > 1.001 !== (scale > 1.001)) {
      target.rx = 0;
      target.ry = 0;
      // 扫光与信息行微视差也回正——放大态扫光本来就关了（不可见），
      // 信息行则别停在半路偏移的位置上
      target.gx = 50;
      target.gy = 50;
    }
    // 位移上限 = 图片比框大出来的部分，两边各一半。还没铺满时框正好贴着图片，
    // 上限为 0 —— 整张都看得见，本来也没得拖。
    const frame = frameSize(next);
    const maxX = Math.max(0, (fit.w * next - frame.w) / 2);
    const maxY = Math.max(0, (fit.h * next - frame.h) / 2);
    target.scale = next;
    target.tx = clamp(nextTx, -maxX, maxX);
    target.ty = clamp(nextTy, -maxY, maxY);
    start();
  }

  function pan(nextTx: number, nextTy: number): void {
    apply(scale, nextTx, nextTy);
  }

  function resetZoom(): void {
    apply(1, 0, 0);
  }

  /**
   * 「100% ↔ 放大」那一步，显示框中心的纵向位移。
   *
   * 100% 时竖排栈（图片 + 信息行）在内容盒里垂直居中，放大时栈正好铺满内容盒，
   * 两个中心相差 `PAD_TOP / 2`（16px 上内边距 → 8px）。锚点里漏掉它，放大后
   * 光标下的画面会整体偏一点；横向两态中心一致，不用管。
   */
  function centerShiftY(next: number): number {
    const wasZoomed = scale > 1.001;
    const willZoom = next > 1.001;
    if (wasZoomed === willZoom) return 0;
    return willZoom ? PAD_TOP / 2 : -PAD_TOP / 2;
  }

  /**
   * 以某个屏幕点为锚缩放：该点下面的画面内容保持不动。
   *
   * 显示框中心在屏幕上的位置是 `center`，内容坐标 p 的屏幕位置是
   * `center + t + p * s`（s 是**真正画出来的**倍率）。要让 p 不动，解出新的 `t`。
   */
  function zoomAt(nextScale: number, clientX: number, clientY: number): void {
    const node = shot;
    if (!node) {
      apply(nextScale, tx, ty);
      return;
    }

    const next = clamp(nextScale, 1, MAX_ZOOM);
    const rect = node.getBoundingClientRect();
    // 显示框本身没被变换过（位移和缩放都加在里面那张 <img> 上），所以它的矩形
    // 中心就是「未变换时的中心」—— 这里**不能**再减 tx / ty，那会把位移算两遍。
    const centerX = rect.left + rect.width / 2;
    const centerY = rect.top + rect.height / 2;

    // 位移和缩放都加在里面那张 <img> 上、以它的中心为原点，所以「让光标下那点
    // 不动」解出来就是 `t' = d - (d - t) * k`，k 是这次缩放前后真实倍率之比。
    const k = next / scale;
    const dx = clientX - centerX;
    const dy = clientY - centerY + centerShiftY(next);
    apply(next, dx - (dx - tx) * k, dy - (dy - ty) * k);
  }

  /** 以图片中心为锚缩放，供按钮与键盘用。 */
  function zoomBy(factor: number): void {
    apply(scale * factor, tx * factor, ty * factor);
  }

  function onwheel(event: WheelEvent): void {
    event.preventDefault();
    // 触控板的横向滚动不该触发缩放
    if (Math.abs(event.deltaY) < 1) return;
    const step = Math.exp(-event.deltaY * WHEEL_SENSITIVITY);
    // 从「适应窗口」起步的第一格给足一点（直接到 `FIRST_STEP`）：一滚轮通常是
    // 想放大看，多走一步更快到位；之后每格都是细步，便于微调。缩小不受影响。
    const factor = step > 1 && scale <= 1.001 ? FIRST_STEP / scale : step;
    zoomAt(scale * factor, event.clientX, event.clientY);
  }

  function onpointerdown(event: PointerEvent): void {
    // 记下「按下时真正点在哪」，供 click / dblclick 判断（理由见 `isBlank`）。
    // 必须在 `setPointerCapture` **之前**取 —— 捕获之后这一串鼠标事件的 target
    // 会被改写成舞台。
    pressedOnBlank = isBlank(event.target);
    pressedOnPhoto = event.target instanceof HTMLImageElement;
    // 每次按下都重新计一次「这次算不算拖拽」
    dragged = 0;
    // 按在按钮 / 链接上（翻页箭头、meta 行的画师名等）不进入拖拽：下面的
    // `setPointerCapture` 会把随后的 click 重定向到舞台，控件自己的 onclick
    // 就永远收不到了 —— 工具条用 stopPropagation 挡住了这一手（见标记处），
    // 箭头和 meta 行是舞台的子孙、没有挡，放大态点它们会「毫无反应」。
    // 非放大态本来就不捕获，这行提前返回不影响任何行为。
    if (event.target instanceof Element && event.target.closest("button, a")) return;
    if (!zoomed || event.button !== 0) return;
    dragging = true;
    dragStart = { x: event.clientX, y: event.clientY, tx, ty };
    stage?.setPointerCapture(event.pointerId);
    event.preventDefault();
  }

  function onpointerup(event: PointerEvent): void {
    if (!dragging) return;
    dragging = false;
    if (stage?.hasPointerCapture(event.pointerId)) stage.releasePointerCapture(event.pointerId);
  }

  /**
   * 按下时是否落在「空白」上：舞台，或两个量尺寸用的盒子。
   *
   * 判断必须用 **pointerdown 的 target**，不能用 click 的 —— 放大后
   * `onpointerdown` 会 `setPointerCapture`，浏览器会把随后那串鼠标事件（含
   * `click` / `dblclick`）**全部重定向到被捕获的元素**上。于是「点图片」的 click
   * 也报成「点在舞台上」，`onstageclick` 就顺手把弹窗关了 —— 主人反馈的
   * 「有时候点一下照片就回到主列表」正是这个。
   */
  function isBlank(node: EventTarget | null): boolean {
    return node === stage || node === frame || node === inner;
  }

  function ondoubleclick(event: MouseEvent): void {
    // 只认**图片上**的双击：工具条上连点两下「+」会被浏览器补发 dblclick，
    // 不挡住的话第二下会被当成「双击放大」，倍率直接跳一大截。
    // 判据同样取 pointerdown 的落点（见 `isBlank`）—— 放大后 dblclick 的 target
    // 会被指针捕获改写成舞台，用 `event.target` 判断会让「双击还原」在放大态失效。
    if (!pressedOnPhoto) return;
    if (zoomed) resetZoom();
    // 双击 = **正好铺满**：图片刚好填满整块区域，一点不裁。滚轮一格格爬上去的
    // 终点也是这里，所以「想立刻铺满」用双击、「想微调」用滚轮，各管一段。
    else zoomAt(coverScale(), event.clientX, event.clientY);
  }

  /** 点空白关闭。点在图片或工具条上不算 —— 那些地方有自己的语义。 */
  function onstageclick(): void {
    if (!pressedOnBlank || dragged > DRAG_SLOP) return;
    onclose();
  }

  // ---------------------------------------------------------------------------
  // 全屏
  // ---------------------------------------------------------------------------

  async function applyFullscreen(next: boolean): Promise<void> {
    try {
      await getCurrentWindow().setFullscreen(next);
      fullscreen = next;
      onfullscreen?.(next);
      return;
    } catch {
      // 不在 Tauri 里（比如在浏览器里调样式）就退回浏览器的全屏 API
    }
    try {
      if (next) await document.documentElement.requestFullscreen();
      else if (document.fullscreenElement) await document.exitFullscreen();
      fullscreen = next;
      onfullscreen?.(next);
    } catch {
      // 没有用户手势或平台不支持：静默放弃，不打断看图
    }
  }

  function toggleFullscreen(): void {
    void applyFullscreen(!fullscreen);
  }

  /** 把原图写进系统剪贴板。见 `api.copyImage` 的注释（右键菜单那条路走不通）。 */
  async function copyImage(): Promise<void> {
    if (!copyPath || copyState === "copying") return;
    copyState = "copying";
    try {
      await api.copyImage(copyPath);
      copyState = "done";
      setTimeout(() => {
        if (copyState === "done") copyState = "idle";
      }, 2000);
    } catch (err) {
      copyState = "failed";
      copyError = String(err);
      setTimeout(() => {
        if (copyState === "failed") copyState = "idle";
      }, 4000);
    }
  }

  // ---------------------------------------------------------------------------
  // 生命周期
  // ---------------------------------------------------------------------------

  // 换图：把「已加载 / 失败 / 缩放 / 姿态」全部归零。
  // 放在 $effect 而不是 `{#key}` 里，父组件就不必记得给组件加 key。
  // 首次打开时把特效开关读回来
  $effect(() => {
    void ensureEffectsLoaded();
  });

  $effect(() => {
    void src;
    untrack(() => {
      loaded = false;
      failed = false;
      natural = { w: 0, h: 0 };
      rx = 0;
      ry = 0;
      gx = 50;
      gy = 50;
      scale = 1;
      tx = 0;
      ty = 0;
      target = { rx: 0, ry: 0, gx: 50, gy: 50, scale: 1, tx: 0, ty: 0 };
      zoomPercent = 100;
      zoomed = false;
      if (frameId) cancelAnimationFrame(frameId);
      frameId = 0;
      paint();
    });
  });

  // 量两块区域。窗口缩放、全屏、侧栏收起、信息行出现/消失都会改它们。
  $effect(() => {
    const outer = frame;
    const innerNode = inner;
    if (!outer || !innerNode) return;
    const measure = () => {
      // 外层没有 padding，clientWidth/Height 直接就是「整块」
      full = { w: outer.clientWidth, h: outer.clientHeight };
      // **内层的 clientWidth/Height 是含 padding 的**，必须自己扣掉才是可用区域。
      // 忘了扣的后果很直观：图片按整块算高度，直接压到工具条上、信息行被挤出画面。
      const style = getComputedStyle(innerNode);
      const padX =
        parseFloat(style.paddingLeft) + parseFloat(style.paddingRight);
      const padY = parseFloat(style.paddingTop) + parseFloat(style.paddingBottom);
      padBottom = parseFloat(style.paddingBottom);
      box = {
        w: Math.max(0, innerNode.clientWidth - padX),
        h: Math.max(0, innerNode.clientHeight - padY),
      };
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(outer);
    observer.observe(innerNode);
    return () => observer.disconnect();
  });

  // 可用区域变了，位移上限也跟着变，把当前位移重新夹一次
  $effect(() => {
    void fit.w;
    void fit.h;
    untrack(() => {
      apply(target.scale, target.tx, target.ty);
      // 尺寸变了但倍率没变时 rAF 不会跑，这里补一次绘制
      if (!frameId) paint();
    });
  });

  $effect(() => {
    const onkeydown = (event: KeyboardEvent) => {
      // 焦点在表单控件里时不抢键盘（弹窗里有标签 / 相册的下拉框）
      const target = event.target as HTMLElement | null;
      const typing =
        target instanceof HTMLInputElement ||
        target instanceof HTMLSelectElement ||
        target instanceof HTMLTextAreaElement ||
        target?.isContentEditable === true;

      if (event.key === "Escape") {
        // 先退出放大，再退出弹窗 —— 否则放大状态下按 Esc 会直接关掉
        if (zoomed) resetZoom();
        else onclose();
        return;
      }
      if (typing) return;

      switch (event.key) {
        case "ArrowLeft":
          onnavigate?.(-1);
          break;
        case "ArrowRight":
          onnavigate?.(1);
          break;
        case "+":
        case "=":
          zoomBy(1.5);
          break;
        case "-":
        case "_":
          zoomBy(1 / 1.5);
          break;
        case "0":
          resetZoom();
          break;
        case "1":
        case "2":
        case "3":
        case "4":
        case "5":
          // 数字键直接评分 —— 表单控件里的输入已被上面的 typing 守卫拦下
          onrate?.(Number(event.key));
          break;
        case "f":
        case "F":
          toggleFullscreen();
          break;
        default:
          return;
      }
      event.preventDefault();
    };

    // 窗口切到后台时别再算，切回来先把姿态归零
    const onvisibility = () => {
      if (document.visibilityState === "visible") {
        // 回到前台重新校准中性角（握持姿势多半变了），姿态归零
        gyroCalibrated = false;
        resetTilt();
      }
    };

    window.addEventListener("keydown", onkeydown);
    document.addEventListener("visibilitychange", onvisibility);

    // 粗指针设备：重力感应接管光锥——卡片随手机倾斜（刚体感），
    // 高光当固定环境光反向移，信息行微视差从同一组 nx/ny 推出。
    // 两条传感管线，按「真的出货」择优，不能只看构造器存不存在：
    // - GravitySensor（Generic Sensor）：数据质量最好（融合后的纯重力矢量），
    //   但部分 WebView（实测小米 Chrome 153）**有构造器、start() 不报错、
    //   reading 却永远不来**——起步 700ms 内一条读数都没等到（或 start 抛错 /
    //   派了 error）就切下面的备选；
    // - deviceorientation：现代 WebView 其实都会派发（36Hz+），旧 WebView 才
    //   缺失——作为兜底管线。
    let gyroHandler: ((event: DeviceOrientationEvent) => void) | null = null;
    let gravitySensor: GravitySensorLike | null = null;
    let gravityFallbackTimer: ReturnType<typeof setTimeout> | null = null;

    // 两条管线都归一化到「右倾为正的 roll + 与 beta 同向的 pitch」再落目标，
    // 中性角自动校准：首次读数时的握持姿态就是 0，回前台时重校。
    const applyPose = (roll: number, pitch: number): void => {
      if (!gyroCalibrated) {
        neutralRoll = roll;
        neutralPitch = pitch;
        gyroCalibrated = true;
      }
      const nx = clamp((roll - neutralRoll) / GYRO_RANGE, -1, 1);
      const ny = clamp((pitch - neutralPitch) / GYRO_RANGE, -1, 1);
      target.rx = ny * GYRO_MAX_TILT;
      target.ry = -nx * GYRO_MAX_TILT;
      target.gx = 50 - nx * 50;
      target.gy = 50 - ny * 50;
      start();
    };

    const startOrientationPipeline = (): void => {
      // 只有前一条管线确认没戏（超时 / 报错 / 不可用）才会走到这里，别叠监听
      if (gyroHandler || gravitySensor) return;
      if (typeof window.DeviceOrientationEvent !== "function") return;
      gyroHandler = (event: DeviceOrientationEvent): void => {
        if (event.beta === null || event.gamma === null) return;
        if (zoomed || !effectsEnabled) return;
        // 屏幕旋转时 beta/gamma 的轴含义互换——按 screen.orientation.angle 重映射
        const angle = screen.orientation?.angle ?? 0;
        let beta = event.beta;
        let gamma = event.gamma;
        if (angle === 90) {
          [beta, gamma] = [-gamma, beta];
        } else if (angle === 270 || angle === -90) {
          [beta, gamma] = [gamma, -beta];
        } else if (angle === 180) {
          [beta, gamma] = [-beta, -gamma];
        }
        applyPose(gamma, beta);
      };
      window.addEventListener("deviceorientation", gyroHandler);
    };

    const abandonGravity = (sensor: GravitySensorLike): void => {
      if (gravitySensor !== sensor) return;
      gravitySensor = null;
      sensor.stop();
      startOrientationPipeline();
    };

    // DOM lib 没有 GravitySensor 的类型——就地取构造器（WebView 有实现）
    const GravitySensorCtor = (
      window as unknown as {
        GravitySensor?: new (options?: { frequency?: number }) => GravitySensorLike;
      }
    ).GravitySensor;
    if (touchPrimary && !calm && GravitySensorCtor) {
      const sensor = new GravitySensorCtor({ frequency: 30 });
      gravitySensor = sensor;
      let gotReading = false;
      sensor.addEventListener("reading", () => {
        if (gravitySensor !== sensor) return; // 已被判死刑，迟到的读数作废
        const gx = sensor.gravity?.x ?? null;
        const gy = sensor.gravity?.y ?? null;
        const gz = sensor.gravity?.z ?? null;
        // 第二种坏法：reading 照派、gravity 却是 null/全零（同一台机器两种
        // 都见过）——无效数据视同管线死亡，一样切 deviceorientation。
        // 静止时重力模长 ≈ 9.8，0.5 以下不可能是真的重力。
        if (
          gx === null ||
          gy === null ||
          gz === null ||
          Math.abs(gx) + Math.abs(gy) + Math.abs(gz) < 0.5
        ) {
          abandonGravity(sensor);
          return;
        }
        if (!gotReading) {
          gotReading = true;
          if (gravityFallbackTimer) {
            clearTimeout(gravityFallbackTimer);
            gravityFallbackTimer = null;
          }
        }
        if (zoomed || !effectsEnabled) return;
        // 由重力矢量在设备坐标系的分量反解姿态角（右倾为正）。pitch 用
        // atan2(gz, -gy) 而非取反 gz——和 deviceorientation 的 beta 同向
        // （直立 ≈ 0，往后放平一路变小），切管线时手感才一致。
        const roll = (Math.atan2(gx, -gy) * 180) / Math.PI;
        const pitch = (Math.atan2(gz, -gy) * 180) / Math.PI;
        applyPose(roll, pitch);
      });
      sensor.addEventListener("error", () => abandonGravity(sensor));
      try {
        sensor.start();
        gravityFallbackTimer = setTimeout(() => {
          gravityFallbackTimer = null;
          if (!gotReading) abandonGravity(sensor);
        }, 700);
      } catch {
        abandonGravity(sensor);
      }
    } else if (touchPrimary && !calm) {
      startOrientationPipeline();
    }

    return () => {
      window.removeEventListener("keydown", onkeydown);
      document.removeEventListener("visibilitychange", onvisibility);
      if (gyroHandler) window.removeEventListener("deviceorientation", gyroHandler);
      if (gravityFallbackTimer) clearTimeout(gravityFallbackTimer);
      gravitySensor?.stop();
      if (frameId) cancelAnimationFrame(frameId);
      frameId = 0;
      // 关掉弹窗就别把整个应用留在全屏里
      if (fullscreen) void applyFullscreen(false);
    };
  });
</script>

<div
  class="viewer"
  class:is-dragging={dragging}
  class:is-zoomed={zoomed}
  class:is-flat={!effectsEnabled}
  bind:this={stage}
  role="presentation"
  onpointermove={onpointermove}
  onpointerleave={onpointerleave}
  onpointerdown={onpointerdown}
  onpointerup={onpointerup}
  onpointercancel={onpointerup}
  onwheel={onwheel}
  ondblclick={ondoubleclick}
  onclick={onstageclick}
>
  <!--
    两层盒子各司其职：外层量「去掉留白的整块」，内层量「带留白的区域」。
    放大时显示框从内层的适应尺寸长到外层那么大，留白就是这样被吃掉的。
  -->
  <div class="viewer__frame" bind:this={frame}>
    <div class="viewer__inner" bind:this={inner}>
      {#if failed}
        <div class="viewer__failure">
          <p class="viewer__failure-title">原图加载失败，文件可能已被移动或删除：</p>
          <p class="viewer__failure-path" data-selectable>{failurePath ?? src}</p>
        </div>
      {:else}
        <!-- 竖排：卡片在上，信息行在下。信息**不进 3D 卡片**，
             免得压在图上、跟着倾斜、还要靠深色渐变垫底才看得清。 -->
        <div class="viewer__stack">
          <div class="viewer__scene">
            <div class="viewer__card">
              <!-- 环境光：同一张原图糊开，氛围色跟着画走。filter 是分组属性，
                   只能落在叶子节点上——套到 card 会压平整个 3D -->
              <img
                src={src}
                alt=""
                aria-hidden="true"
                draggable="false"
                class="viewer__ambient"
              />

              <!-- 光锥底：背光场 -->
              <div class="viewer__halo"></div>

              <!-- 主体。圆角裁切只能在这一层，缩放只能在内层的 img 上（见文件头） -->
              <div class="viewer__shot" bind:this={shot}>
                <img
                  {src}
                  {alt}
                  draggable="false"
                  class="viewer__img {loaded ? 'is-loaded' : ''}"
                  style={fit.w > 0 ? `width: ${fit.w}px; height: ${fit.h}px;` : ""}
                  onload={(event) => {
                    const node = event.currentTarget as HTMLImageElement;
                    natural = { w: node.naturalWidth, h: node.naturalHeight };
                    loaded = true;
                  }}
                  onerror={() => (failed = true)}
                />
              </div>

              <!-- 扫光（叶子节点，允许 mix-blend-mode） -->
              <div class="viewer__glare"></div>
              <div class="viewer__sheen"></div>
            </div>
          </div>

          {#if meta}
            <div class="viewer__meta">
              {@render meta()}
            </div>
          {/if}
        </div>
      {/if}
    </div>
  </div>

  {#if !loaded && !failed}
    <div class="viewer__spinner"></div>
  {/if}

  {#if onnavigate && !failed}
    <button type="button" class="viewer__nav left-4" aria-label="上一张" onclick={() => onnavigate(-1)}>
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-5">
        <path stroke-linecap="round" stroke-linejoin="round" d="M15.75 19.5L8.25 12l7.5-7.5" />
      </svg>
    </button>
    <button type="button" class="viewer__nav right-4" aria-label="下一张" onclick={() => onnavigate(1)}>
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-5">
        <path stroke-linecap="round" stroke-linejoin="round" d="M8.25 4.5l7.5 7.5-7.5 7.5" />
      </svg>
    </button>
  {/if}

  {#if !failed}
    <!-- 工具条上的指针事件不要冒泡到舞台，否则按着按钮拖会变成平移图片 -->
    <div
      class="viewer__tools"
      role="toolbar"
      aria-label="查看器工具"
      tabindex="-1"
      onpointerdown={(event) => event.stopPropagation()}
    >
      <button
        type="button"
        class="viewer__tool"
        aria-label="缩小"
        title="缩小 (-)"
        disabled={!zoomed}
        onclick={() => zoomBy(1 / 1.5)}
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
          <path stroke-linecap="round" d="M5 12h14" />
        </svg>
      </button>

      <button
        type="button"
        class="viewer__tool viewer__tool--wide tabular-nums"
        title="适应窗口 (0)"
        onclick={resetZoom}
      >
        {zoomPercent}%
      </button>

      <button
        type="button"
        class="viewer__tool"
        aria-label="放大"
        title="放大 (+)"
        disabled={zoomPercent >= MAX_ZOOM * 100}
        onclick={() => zoomBy(1.5)}
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
          <path stroke-linecap="round" d="M12 5v14M5 12h14" />
        </svg>
      </button>

      <span class="viewer__divider"></span>

      {#if copyPath}
        <button
          type="button"
          class="viewer__tool {copyState === 'done' ? 'is-on' : ''} {copyState === 'failed'
            ? 'is-failed'
            : ''}"
          aria-label={copyState === "done" ? "已复制" : "复制图片"}
          title={copyState === "done"
            ? "已复制到剪贴板"
            : copyState === "failed"
              ? `复制失败：${copyError}`
              : "复制图片到剪贴板"}
          onclick={copyImage}
        >
          {#if copyState === "done"}
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" class="size-4">
              <path stroke-linecap="round" stroke-linejoin="round" d="M4.5 12.75l6 6 9-13.5" />
            </svg>
          {:else if copyState === "failed"}
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
              <path stroke-linecap="round" stroke-linejoin="round" d="M12 9v4m0 4h.01M10.3 3.9L1.8 18a2 2 0 001.7 3h17a2 2 0 001.7-3L13.7 3.9a2 2 0 00-3.4 0z" />
            </svg>
          {:else}
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
              <path
                stroke-linecap="round"
                stroke-linejoin="round"
                d="M8 7V5a2 2 0 012-2h8a2 2 0 012 2v8a2 2 0 01-2 2h-2M6 8h8a2 2 0 012 2v8a2 2 0 01-2 2H6a2 2 0 01-2-2v-8a2 2 0 012-2z"
              />
            </svg>
          {/if}
        </button>
      {/if}

      <button
        type="button"
        class="viewer__tool {effectsEnabled ? 'is-on' : ''}"
        aria-label={effectsEnabled ? "关闭光锥特效" : "开启光锥特效"}
        aria-pressed={effectsEnabled}
        title={effectsEnabled ? "关闭光锥特效（鼠标倾斜 + 扫光）" : "开启光锥特效"}
        onclick={toggleEffects}
      >
        {#if effectsEnabled}
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              d="M12 3v2m0 14v2M5.6 5.6l1.4 1.4m10 10l1.4 1.4M3 12h2m14 0h2M5.6 18.4l1.4-1.4m10-10l1.4-1.4"
            />
            <circle cx="12" cy="12" r="3.5" />
          </svg>
        {:else}
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              d="M12 3v2m0 14v2M5.6 5.6l1.4 1.4m10 10l1.4 1.4M3 12h2m14 0h2M5.6 18.4l1.4-1.4m10-10l1.4-1.4"
            />
            <circle cx="12" cy="12" r="3.5" />
            <path stroke-linecap="round" d="M4 20L20 4" />
          </svg>
        {/if}
      </button>

      <button
        type="button"
        class="viewer__tool {fullscreen ? 'is-on' : ''}"
        aria-label={fullscreen ? "退出全屏" : "全屏"}
        aria-pressed={fullscreen}
        title={fullscreen ? "退出全屏 (F)" : "全屏 (F)"}
        onclick={toggleFullscreen}
      >
        {#if fullscreen}
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              d="M9 3v4a2 2 0 01-2 2H3m12-6v4a2 2 0 002 2h4M9 21v-4a2 2 0 00-2-2H3m12 6v-4a2 2 0 012-2h4"
            />
          </svg>
        {:else}
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-4">
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              d="M3 9V5a2 2 0 012-2h4M15 3h4a2 2 0 012 2v4m0 6v4a2 2 0 01-2 2h-4M9 21H5a2 2 0 01-2-2v-4"
            />
          </svg>
        {/if}
      </button>

      {#if actions}
        {@render actions()}
      {/if}
    </div>
  {/if}
</div>

<style lang="postcss">
  @reference "../../app.css";

  /* ---- 舞台：perspective 只放这里 ------------------------------------ */
  .viewer {
    position: absolute;
    inset: 0;
    /* 大图约 60vh 高，1.5× 是「能明显看出景深但不变形」的档位 */
    perspective: 900px;
    perspective-origin: 50% 50%;
  }

  .viewer.is-dragging {
    cursor: grabbing;
  }

  /* 外层：clientWidth/Height 就是「去掉留白的整块」 */
  .viewer__frame {
    width: 100%;
    height: 100%;
  }

  /* 内层：padding 在这一层，clientWidth/Height 就是「带留白的区域」 */
  .viewer__inner {
    display: grid;
    width: 100%;
    height: 100%;
    padding: 1rem 1rem 4.5rem;
    place-items: center;
  }

  /*
    放大时把左右和上边的留白也吃掉，只留底部那截给工具条。

    只把显示框的尺寸算成「整块」是不够的 —— 承载它的 `.viewer__inner` 还带着
    padding，框比内容盒宽，居中时会被挤出一侧，观感就是「放大了但四周还留一圈背景」。
    尺寸和容器要一起改，留白才算真的吃掉。
  */
  .viewer.is-zoomed .viewer__inner {
    padding: 0 0 4.5rem 0;
  }

  /* 竖排：卡片在上，信息行在下 */
  .viewer__stack {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.5rem;
  }

  /* ---- 入场：缩放 + 轻微翻转。与鼠标倾斜分开，互不打架 --------------- */
  .viewer__scene {
    animation: scene-in 0.42s cubic-bezier(0.22, 0.61, 0.36, 1) both;
  }

  @keyframes scene-in {
    from {
      opacity: 0;
      transform: scale(0.93) rotateX(9deg);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }

  /* ---- 卡片：preserve-3d 的载体 -------------------------------------- */
  .viewer__card {
    position: relative;
    transform-style: preserve-3d;
    transform: rotateX(var(--rx)) rotateY(var(--ry));
    will-change: transform;
  }

  /* ---- 环境光：原图糊开 ----------------------------------------------
     氛围色取自画本身而不是配色方案——每换一张图，光锥的环境色跟着走。
     blur 是分组属性，只能待在叶子节点上；inset 外扩 + scale 是为了把
     blur 在边缘吃出来的透明圈藏到卡片后面。
     mask 是这道光的命门：糊开的原图仍是一张「矩形的鬼影」，不遮的话它
     的边界（以及被窗口边缘 / 信息面板切断处）会显出一圈色彩断层——
     椭圆遮罩让它贴着卡片后方保持满强度、往外一路溶成零，光晕在抵达
     任何边界之前就化没了。mask 同样是分组属性，照样只能待在叶子上。 */
  .viewer__ambient {
    position: absolute;
    inset: -8%;
    object-fit: cover;
    transform: translateZ(-64px) scale(1.2);
    filter: blur(48px) saturate(1.25);
    mask-image: radial-gradient(50% 50% at 50% 50%, #000 60%, transparent 96%);
    -webkit-mask-image: radial-gradient(50% 50% at 50% 50%, #000 60%, transparent 96%);
    pointer-events: none;
  }

  /* ---- 光锥底 -------------------------------------------------------- */
  .viewer__halo {
    position: absolute;
    inset: -14%;
    transform: translateZ(-96px);
    border-radius: 50%;
    background: radial-gradient(
      circle at 50% 50%,
      color-mix(in oklab, var(--tone-accent) 34%, transparent) 0%,
      color-mix(in oklab, var(--tone-accent) 12%, transparent) 42%,
      transparent 72%
    );
    filter: blur(18px);
    pointer-events: none;
  }

  /* ---- 原图：圆角裁切在这一层（不能上移到卡片，会压平 3D） ---------- */
  .viewer__shot {
    position: relative;
    overflow: hidden;
    border-radius: var(--radius-card);
    box-shadow: 0 24px 70px -20px rgb(0 0 0 / 0.75);
    /* 入场那一下的色差脉冲。不用 forwards，免得动画结束后常驻一个 filter 图层 */
    animation: rgb-pulse 0.62s cubic-bezier(0.22, 0.61, 0.36, 1) 1;
    cursor: zoom-in;
  }

  .viewer.is-dragging .viewer__shot {
    cursor: grabbing;
  }

  /* abx15 的 RGB 偏移是 shader 里 R 右移 / G 左移各 1% UV，且只在中途最强。
     这里用两层 drop-shadow 等效：不透明矩形上的偏移阴影只在边缘露出一条色边。 */
  @keyframes rgb-pulse {
    0% {
      filter: drop-shadow(0 0 0 rgb(255 0 64 / 0)) drop-shadow(0 0 0 rgb(0 255 255 / 0));
    }
    45% {
      filter: drop-shadow(3px 0 0 rgb(255 0 64 / 0.45))
        drop-shadow(-3px 0 0 rgb(0 255 255 / 0.38));
    }
    100% {
      filter: drop-shadow(0 0 0 rgb(255 0 64 / 0)) drop-shadow(0 0 0 rgb(0 255 255 / 0));
    }
  }

  /*
    图片的 width / height 由 JS 写成「适应尺寸」（fit），显示框的 width / height
    也由 JS 写（放大时会从 fit 长到整块）。两者都不走 CSS 百分比 —— 百分比
    max-height 在高度 auto 的祖先链里会失效，而这里的每一像素都要算得准。
    `inset: 0; margin: auto` 把图片在显示框里居中；transform 以自身中心为原点缩放。
  */
  .viewer__img {
    position: absolute;
    inset: 0;
    margin: auto;
    display: block;
    object-fit: contain;
    opacity: 0;
    transition: opacity 0.28s ease-out;
    backface-visibility: hidden;
    /* translate 在前、scale 在后：位移走屏幕像素，不受倍率影响 */
    transform: translate3d(var(--tx), var(--ty), 0) scale(var(--s));
    will-change: transform;
  }

  .viewer__img.is-loaded {
    opacity: 1;
  }

  /* ---- 扫光：跟随鼠标的镜面高光（叶子） ------------------------------ */
  /* 放大后关掉两片 mix-blend-mode 扫光。两个理由：
     一是 blend mode 要对整块区域重新合成，放大时那是最贵的一笔开销；
     二是盯着细节看的时候，一层镜面高光本来就碍事。 */
  .viewer.is-zoomed .viewer__glare,
  .viewer.is-zoomed .viewer__sheen {
    display: none;
  }

  .viewer__glare {
    position: absolute;
    inset: 0;
    transform: translateZ(24px);
    border-radius: var(--radius-card);
    background: radial-gradient(
      farthest-corner circle at var(--gx) var(--gy),
      rgb(255 255 255 / 0.5) 0%,
      rgb(255 255 255 / 0.16) 26%,
      rgb(0 0 0 / 0.32) 88%
    );
    mix-blend-mode: overlay;
    opacity: 0.85;
    pointer-events: none;
  }

  /* ---- 扫光：一道斜光随鼠标反向扫过（叶子，color-dodge） ------------- */
  .viewer__sheen {
    position: absolute;
    inset: 0;
    overflow: hidden;
    border-radius: var(--radius-card);
    transform: translateZ(26px);
    pointer-events: none;
  }

  .viewer__sheen::after {
    content: "";
    position: absolute;
    inset: 0;
    mix-blend-mode: color-dodge;
    background-image: linear-gradient(
      105deg,
      transparent 38%,
      rgb(255 255 255 / 0.42) 50%,
      transparent 62%
    );
    background-size: 220% 100%;
    /* 鼠标往右，光带往左扫 —— 反向滑动才有「扫过」的感觉 */
    background-position: calc(100% - var(--gx) * 2) 0;
    opacity: 0.7;
  }

  /* ---- 信息行：在图片**下面**，不在图片上 ---------------------------- */
  /*
    之前它是卡片里的一层（translateZ(64px) + 底部黑色渐变），压在图上。
    两个毛病：一是挡住画面，二是文字颜色跟着主题走 —— 浅色主题下就成了
    深字压在深色渐变上，读不出来。挪到普通流里、用主题自己的前景色，
    两个问题一起解决。
  */
  .viewer__meta {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    width: 100%;
    /* 高度必须固定：`fit` 的可用高度里按这个常量扣掉了它 */
    height: 1.875rem;
    /* 微视差：paint() 每帧写 --meta-x/y（沿鼠标反方向 ±6px）。只平移不旋转
       —— 「meta 不进 3D」的约束不破，空间感照样有 */
    translate: var(--meta-x, 0) var(--meta-y, 0);
    pointer-events: none;
  }

  /* `meta` snippet 的标记是在父组件里编译的，带的是父组件的 scoped 哈希，
     所以这两个类只能用 :global 选到 —— 它们是这个组件对外的样式契约。 */
  .viewer__meta :global(.viewer-chip) {
    flex: none;
    border-radius: 0.25rem;
    background: color-mix(in oklab, var(--tone-accent) 20%, transparent);
    padding: 0.1rem 0.4rem;
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--tone-accent);
  }

  /*
    可点击的画师名。信息行整体是 `pointer-events: none`（免得挡住下面的图），
    所以这一个要单独把事件收回来。
  */
  .viewer__meta :global(.viewer-artist) {
    pointer-events: auto;
    flex: none;
    cursor: pointer;
    border: 1px solid transparent;
    border-radius: 0.25rem;
    padding: 0.1rem 0.4rem;
    font-size: 11px;
    color: var(--tone-accent);
    transition:
      background 0.15s,
      border-color 0.15s;
  }

  .viewer__meta :global(.viewer-artist:hover) {
    border-color: color-mix(in oklab, var(--tone-accent) 45%, transparent);
    background: color-mix(in oklab, var(--tone-accent) 12%, transparent);
  }

  /* 标题带 data-selectable：恢复指针事件，让它能选中复制 —— 也顺手治好
     「点标题会关弹窗」（没有这一层，点击会穿透到 inner 被判成空白） */
  .viewer__meta :global([data-selectable]) {
    pointer-events: auto;
  }

  .viewer__meta :global(.viewer-caption) {
    min-width: 0;
    max-width: 60vw;
    overflow: hidden;
    font-family: var(--font-mono);
    font-size: 11px;
    /* 主题的前景弱化色，浅色主题下是深色、深色主题下是浅色，两边都读得清 */
    color: var(--tone-fg-soft);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* ---- 翻页箭头 ------------------------------------------------------ */
  .viewer__nav {
    position: absolute;
    top: 50%;
    translate: 0 -50%;
    cursor: pointer;
    border-radius: 9999px;
    background: rgb(0 0 0 / 0.5);
    padding: 0.5rem;
    color: rgb(255 255 255 / 0.8);
    backdrop-filter: blur(4px);
    transition:
      background 0.15s,
      color 0.15s,
      opacity 0.2s;
  }

  /* 放大是看细节的时刻：箭头淡到三成、悬停恢复 —— 别压在画面上 */
  .viewer.is-zoomed .viewer__nav {
    opacity: 0.3;
  }

  .viewer__nav:hover {
    background: rgb(0 0 0 / 0.72);
    color: white;
    opacity: 1;
  }

  /* ---- 底部工具条 ---------------------------------------------------- */
  .viewer__tools {
    position: absolute;
    bottom: 1rem;
    left: 50%;
    translate: -50% 0;
    display: flex;
    align-items: center;
    gap: 0.25rem;
    border-radius: 9999px;
    border: 1px solid var(--tone-line-faint);
    background: color-mix(in oklab, var(--tone-panel) 88%, transparent);
    padding: 0.25rem;
    box-shadow: 0 10px 30px -10px rgb(0 0 0 / 0.6);
    backdrop-filter: blur(12px);
  }

  .viewer__tool {
    display: flex;
    cursor: pointer;
    align-items: center;
    justify-content: center;
    border-radius: 9999px;
    padding: 0.375rem;
    color: var(--tone-fg-soft);
    transition:
      background 0.15s,
      color 0.15s;
  }

  .viewer__tool:hover:not(:disabled) {
    background: var(--tone-case);
    color: var(--tone-fg);
  }

  .viewer__tool:disabled {
    cursor: not-allowed;
    opacity: 0.35;
  }

  .viewer__tool--wide {
    padding-inline: 0.5rem;
    font-family: var(--font-mono);
    font-size: 11px;
  }

  .viewer__tool.is-on {
    background: color-mix(in oklab, var(--tone-accent) 22%, transparent);
    color: var(--tone-accent);
  }

  /* 复制失败：按钮自己变红，别静默 —— 主人反馈过「点了没反应」最难查 */
  .viewer__tool.is-failed {
    background: color-mix(in oklab, #e11d48 22%, transparent);
    color: #e11d48;
  }

  .viewer__divider {
    width: 1px;
    height: 1rem;
    margin-inline: 0.125rem;
    background: var(--tone-line-faint);
  }

  /* ---- 加载指示 ------------------------------------------------------ */
  .viewer__spinner {
    position: absolute;
    top: 50%;
    left: 50%;
    width: 2.5rem;
    height: 2.5rem;
    margin: -1.25rem 0 0 -1.25rem;
    animation: spin 0.9s linear infinite;
    border: 2px solid var(--tone-line-faint);
    border-top-color: var(--tone-accent);
    border-radius: 50%;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* ---- 加载失败 ------------------------------------------------------ */
  .viewer__failure {
    max-width: 36rem;
    text-align: center;
  }

  .viewer__failure-title {
    font-size: 0.875rem;
    color: var(--tone-fg-soft);
  }

  .viewer__failure-path {
    margin-top: 0.5rem;
    font-family: var(--font-mono);
    font-size: 0.75rem;
    word-break: break-all;
    color: var(--tone-text-muted);
  }

  /* ---- 特效总开关：关掉后变成朴素平面显示 ---------------------------- */
  .viewer.is-flat .viewer__halo,
  .viewer.is-flat .viewer__ambient,
  .viewer.is-flat .viewer__glare,
  .viewer.is-flat .viewer__sheen {
    display: none;
  }

  .viewer.is-flat .viewer__meta {
    translate: 0 0;
  }

  .viewer.is-flat .viewer__scene,
  .viewer.is-flat .viewer__shot {
    animation: none;
  }

  .viewer.is-flat .viewer__card {
    transform: none;
  }

  /* ---- 尊重系统的减少动效设置 ---------------------------------------- */
  @media (prefers-reduced-motion: reduce) {
    .viewer__scene,
    .viewer__shot,
    .viewer__spinner {
      animation: none;
    }

    .viewer__card {
      transform: none;
    }

    .viewer__glare,
    .viewer__sheen {
      display: none;
    }
  }
</style>

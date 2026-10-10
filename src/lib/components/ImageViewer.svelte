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
   *     .viewer__frame     量尺寸用的盒子，卡片在它里面居中
   *       .viewer__scene   入场动画（缩放 + 轻微翻转）
   *         .viewer__card  preserve-3d，跟鼠标做 rotateX / rotateY
   *           .viewer__halo   translateZ(-64px)  光锥底（背光场）
   *           .viewer__shot   translateZ(0)      原图（圆角裁切在这层）
   *           .viewer__glare  translateZ(24px)   扫光（overlay）
   *           .viewer__sheen  translateZ(26px)   斜光（color-dodge）
   *           .viewer__meta   translateZ(64px)   前景信息
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
    /** 加载失败时展示的路径，方便主人自己去磁盘上找 */
    failurePath?: string;
  }

  let {
    src,
    alt,
    meta,
    actions,
    onclose,
    onnavigate,
    onfullscreen,
    failurePath,
  }: Props = $props();

  /** 最大倾角。卡牌类效果常用 ±14°，检视大图收敛到 7° 更稳。 */
  const MAX_TILT = 7;
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
  /** 滚轮灵敏度：deltaY 每 1px 乘 exp(0.0015)，一格标准滚轮约 ×1.19。 */
  const WHEEL_SENSITIVITY = 0.0015;
  /** 双击放到的倍率。 */
  const DOUBLE_CLICK_ZOOM = 2.5;
  /** 拖拽超过这个距离就不算「点击」，避免拖完顺手把弹窗关了。 */
  const DRAG_SLOP = 5;

  let stage = $state<HTMLElement | null>(null);
  let frame = $state<HTMLElement | null>(null);
  let shot = $state<HTMLElement | null>(null);

  let loaded = $state(false);
  let failed = $state(false);
  let fullscreen = $state(false);

  /** 原图像素尺寸，`onload` 时才拿得到。 */
  let natural = $state({ w: 0, h: 0 });
  /** `.viewer__frame` 的内容区尺寸，由 ResizeObserver 喂。 */
  let box = $state({ w: 0, h: 0 });

  /** 缩放的显示尺寸。**只缩不放**：小图保持原尺寸，免得糊。 */
  let fit = $derived.by(() => {
    if (!natural.w || !natural.h || !box.w || !box.h) return { w: 0, h: 0 };
    const k = Math.min(box.w / natural.w, box.h / natural.h, 1);
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

  /** 显示用的倍率，只在整数变化时才写，避免每帧重渲染。 */
  let zoomPercent = $state(100);
  let zoomed = $state(false);

  let dragging = $state(false);
  let dragStart = { x: 0, y: 0, tx: 0, ty: 0 };
  let dragged = 0;

  /** 系统是否要求减少动效。 */
  const calm = $derived(
    typeof window !== "undefined" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  );

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

    // 放大之后鼠标负责平移，再叠倾斜就晕了
    const node = frame;
    if (!node || calm || zoomed) return;

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
    }
    const shotNode = shot;
    if (shotNode) {
      shotNode.style.setProperty("--s", String(scale));
      shotNode.style.setProperty("--tx", `${tx}px`);
      shotNode.style.setProperty("--ty", `${ty}px`);
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
   * 一次性写入缩放与位移，并把位移夹在「图片边缘不越过裁切框」的范围内。
   *
   * 图片在倍率 s 下比裁切框大出的部分是 `fit * (s - 1)`，两边各分一半，
   * 所以单侧的最大位移就是 `fit * (s - 1) / 2`。
   */
  function apply(nextScale: number, nextTx: number, nextTy: number): void {
    const next = clamp(nextScale, 1, MAX_ZOOM);
    const maxX = (fit.w * (next - 1)) / 2;
    const maxY = (fit.h * (next - 1)) / 2;
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
   * 以某个屏幕点为锚缩放：该点下面的画面内容保持不动。
   *
   * 图片中心在屏幕上的位置是 `center`，内容坐标 p 的屏幕位置是
   * `center + t + p * s`。要让 p 不动，解出新的 `t` 即可。
   */
  function zoomAt(nextScale: number, clientX: number, clientY: number): void {
    const node = shot;
    if (!node) {
      apply(nextScale, tx, ty);
      return;
    }
    const rect = node.getBoundingClientRect();
    // 位移不改变盒子中心、缩放以中心为原点，所以减去 tx / ty 就是未变换时的中心
    const centerX = rect.left + rect.width / 2 - tx;
    const centerY = rect.top + rect.height / 2 - ty;

    const next = clamp(nextScale, 1, MAX_ZOOM);
    const k = next / scale;
    const dx = clientX - centerX;
    const dy = clientY - centerY;
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
    zoomAt(scale * Math.exp(-event.deltaY * WHEEL_SENSITIVITY), event.clientX, event.clientY);
  }

  function onpointerdown(event: PointerEvent): void {
    if (!zoomed || event.button !== 0) return;
    dragging = true;
    dragged = 0;
    dragStart = { x: event.clientX, y: event.clientY, tx, ty };
    stage?.setPointerCapture(event.pointerId);
    event.preventDefault();
  }

  function onpointerup(event: PointerEvent): void {
    if (!dragging) return;
    dragging = false;
    if (stage?.hasPointerCapture(event.pointerId)) stage.releasePointerCapture(event.pointerId);
  }

  function ondoubleclick(event: MouseEvent): void {
    // 只认图片上的双击。工具条上连点两下「+」会被浏览器同时派发 dblclick，
    // 不挡住的话第二下会被当成「双击放大」，倍率直接跳到 2.5×。
    if (!(event.target instanceof HTMLImageElement)) return;
    if (zoomed) resetZoom();
    else zoomAt(DOUBLE_CLICK_ZOOM, event.clientX, event.clientY);
  }

  /** 点空白关闭。点在图片或工具条上不算 —— 那些地方有自己的语义。 */
  function onstageclick(event: MouseEvent): void {
    if (event.target !== stage || dragged > DRAG_SLOP) return;
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

  // ---------------------------------------------------------------------------
  // 生命周期
  // ---------------------------------------------------------------------------

  // 换图：把「已加载 / 失败 / 缩放 / 姿态」全部归零。
  // 放在 $effect 而不是 `{#key}` 里，父组件就不必记得给组件加 key。
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

  // 量可用区域。窗口缩放、全屏、侧栏收起都会改它。
  $effect(() => {
    const node = frame;
    if (!node) return;
    const measure = () => {
      box = { w: node.clientWidth, h: node.clientHeight };
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
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
      if (document.visibilityState === "visible") resetTilt();
    };

    window.addEventListener("keydown", onkeydown);
    document.addEventListener("visibilitychange", onvisibility);

    return () => {
      window.removeEventListener("keydown", onkeydown);
      document.removeEventListener("visibilitychange", onvisibility);
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
  bind:this={stage}
  role="presentation"
  onpointermove={onpointermove}
  onpointerleave={() => {
    if (!dragging) resetTilt();
  }}
  onpointerdown={onpointerdown}
  onpointerup={onpointerup}
  onpointercancel={onpointerup}
  onwheel={onwheel}
  ondblclick={ondoubleclick}
  onclick={onstageclick}
>
  <div class="viewer__frame" bind:this={frame}>
    {#if failed}
      <div class="viewer__failure">
        <p class="viewer__failure-title">原图加载失败，文件可能已被移动或删除：</p>
        <p class="viewer__failure-path" data-selectable>{failurePath ?? src}</p>
      </div>
    {:else}
      <div class="viewer__scene">
        <div class="viewer__card">
          <!-- 光锥底：背光场 -->
          <div class="viewer__halo"></div>

          <!-- 主体。圆角裁切只能在这一层，缩放只能在内层的 img 上（见文件头） -->
          <div
            class="viewer__shot"
            bind:this={shot}
            style={fit.w > 0 ? `width: ${fit.w}px; height: ${fit.h}px;` : ""}
          >
            <img
              {src}
              {alt}
              draggable="false"
              class="viewer__img {loaded ? 'is-loaded' : ''}"
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

          {#if meta}
            <div class="viewer__meta">
              {@render meta()}
            </div>
          {/if}
        </div>
      </div>
    {/if}
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
    /* 下边留出工具条的位置 */
    padding: 1rem 1rem 4.5rem;
    /* 大图约 60vh 高，1.5× 是「能明显看出景深但不变形」的档位 */
    perspective: 900px;
    perspective-origin: 50% 50%;
  }

  .viewer.is-dragging {
    cursor: grabbing;
  }

  /* 量尺寸用的盒子：它的 clientWidth / clientHeight 就是可用区域 */
  .viewer__frame {
    display: grid;
    width: 100%;
    height: 100%;
    place-items: center;
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

  /* ---- 光锥底 -------------------------------------------------------- */
  .viewer__halo {
    position: absolute;
    inset: -14%;
    transform: translateZ(-64px);
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

  /* 尺寸由 JS 算好写在元素 style 上；这里的 100% 只是加载完成前的兜底 */
  .viewer__img {
    display: block;
    width: 100%;
    height: 100%;
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

  /* ---- 前景信息层 ---------------------------------------------------- */
  .viewer__meta {
    position: absolute;
    right: -1px;
    bottom: -1px;
    left: -1px;
    transform: translateZ(64px);
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.4rem 0.6rem;
    border-radius: 0 0 var(--radius-card) var(--radius-card);
    background: linear-gradient(to top, rgb(0 0 0 / 0.72), transparent);
    pointer-events: none;
  }

  /* `meta` snippet 的标记是在父组件里编译的，带的是父组件的 scoped 哈希，
     所以这两个类只能用 :global 选到 —— 它们是这个组件对外的样式契约。 */
  .viewer__meta :global(.viewer-chip) {
    flex: none;
    border-radius: 0.25rem;
    background: color-mix(in oklab, var(--tone-accent) 22%, transparent);
    padding: 0.05rem 0.35rem;
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--tone-accent);
  }

  .viewer__meta :global(.viewer-caption) {
    min-width: 0;
    flex: 1;
    overflow: hidden;
    font-family: var(--font-mono);
    font-size: 10px;
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
      color 0.15s;
  }

  .viewer__nav:hover {
    background: rgb(0 0 0 / 0.72);
    color: white;
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

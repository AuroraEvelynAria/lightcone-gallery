<script lang="ts">
  /**
   * 作品检视弹窗：三层嵌套光锥视差 + 鼠标跟随扫光。纯 CSS 3D，无 WebGL。
   *
   * ## 层结构
   *
   * ```
   * .viewer              固定全屏 + 背幕
   *   .viewer__scene     入场动画（缩放 + 轻微翻转）
   *     .viewer__stage   perspective: 900px
   *       .viewer__card  preserve-3d，跟鼠标做 rotateX / rotateY
   *         .viewer__halo   translateZ(-64px)  光锥底（背光场）
   *         .viewer__shot   translateZ(0)      原图（圆角裁切在这层）
   *         .viewer__glare  translateZ(24px)   扫光（overlay）
   *         .viewer__meta   translateZ(64px)   PID / 页码 / 路径
   * ```
   *
   * ## 一条必须守住的规则
   *
   * `overflow != visible`、`opacity < 1`、`filter`、`mix-blend-mode != normal`、
   * `contain: paint` 都会**把该元素的 `transform-style` 强制降级为 flat**
   * （CSS Transforms L2 的「分组属性」）。所以：
   * - 圆角裁切只能放在最内层的图片上，放卡片容器上会把下面的 3D 层全压平；
   * - `mix-blend-mode` 的扫光必须是**叶子**：它自身可以被 3D 定位，但不能有 3D 子层。
   *
   * ## 参考
   *
   * 层距与阻尼取值参考 `simeydotme/pokemon-cards-css`（同样是 Svelte 项目，
   * 纯 CSS 做鼠标跟随全息高光）。RGB 偏移来自 `abx15/3d-interactive-gallery`，
   * 但那边是 WebGL shader，这里用双层 `drop-shadow` 做等效的入场脉冲。
   */
  import { convertFileSrc } from "@tauri-apps/api/core";

  import type { Artwork } from "$lib/types";

  let { artwork, onclose }: { artwork: Artwork; onclose: () => void } = $props();

  /** 最大倾角。卡牌类效果常用 ±14°，检视大图收敛到 7° 更稳。 */
  const MAX_TILT = 7;
  /** 每帧向目标插值的比例，≈140ms 时间常数。 */
  const EASE = 0.12;
  /** 收敛阈值，低于它就停掉 rAF。 */
  const SETTLE = 0.02;

  let stage = $state<HTMLElement | null>(null);
  let loaded = $state(false);
  let failed = $state(false);

  // 渲染用值（$state：写进 style 才会触发重绘）
  let rx = $state(0);
  let ry = $state(0);
  let gx = $state(50);
  let gy = $state(50);

  // 目标值：pointermove 只改这里，由 rAF 统一插值（高频事件里绝不直接写样式）
  let target = { rx: 0, ry: 0, gx: 50, gy: 50 };
  let frame = 0;

  /** 系统是否要求减少动效。 */
  const calm = $derived(
    typeof window !== "undefined" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  );

  let source = $derived(convertFileSrc(artwork.originalPath));

  function onpointermove(event: PointerEvent): void {
    const node = stage;
    if (!node || calm) return;

    const rect = node.getBoundingClientRect();
    const px = ((event.clientX - rect.left) / rect.width) * 100;
    const py = ((event.clientY - rect.top) / rect.height) * 100;
    // 鼠标在哪一侧，哪一侧就朝自己翻过来（像用手指按住卡片的那一角）
    target = {
      ry: -((px - 50) / 50) * MAX_TILT,
      rx: ((py - 50) / 50) * MAX_TILT,
      gx: px,
      gy: py,
    };
    start();
  }

  function reset(): void {
    target = { rx: 0, ry: 0, gx: 50, gy: 50 };
    start();
  }

  function start(): void {
    if (!frame) frame = requestAnimationFrame(tick);
  }

  function tick(): void {
    rx += (target.rx - rx) * EASE;
    ry += (target.ry - ry) * EASE;
    gx += (target.gx - gx) * EASE;
    gy += (target.gy - gy) * EASE;

    const settled =
      Math.abs(target.rx - rx) < SETTLE &&
      Math.abs(target.ry - ry) < SETTLE &&
      Math.abs(target.gx - gx) < SETTLE &&
      Math.abs(target.gy - gy) < SETTLE;

    // 收敛后停掉，不空转
    frame = settled ? 0 : requestAnimationFrame(tick);
  }

  $effect(() => {
    const onkeydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onclose();
    };
    window.addEventListener("keydown", onkeydown);

    // 窗口切到后台时别再算，切回来先把姿态归零
    const onvisibility = () => {
      if (document.visibilityState === "visible") reset();
    };
    document.addEventListener("visibilitychange", onvisibility);

    return () => {
      window.removeEventListener("keydown", onkeydown);
      document.removeEventListener("visibilitychange", onvisibility);
      if (frame) cancelAnimationFrame(frame);
      frame = 0;
    };
  });
</script>

<div
  class="fixed inset-0 z-50 flex flex-col bg-black/88 backdrop-blur-sm"
  role="dialog"
  aria-modal="true"
  aria-label={`作品 ${artwork.pid} 第 ${artwork.page} 页`}
>
  <!-- 背幕：点空白处关闭 -->
  <button
    type="button"
    class="absolute inset-0 cursor-zoom-out"
    aria-label="关闭"
    onclick={onclose}
  ></button>

  <header class="relative z-10 flex items-center gap-3 px-5 py-3 text-soft">
    <span class="font-mono text-sm font-semibold text-accent-400" title="Pixiv 作品号">
      {artwork.pid}
    </span>
    <span class="text-xs text-muted">p{artwork.page}</span>
    <div class="flex-1"></div>
    <button
      type="button"
      class="cursor-pointer rounded-md border border-surface-700 px-3 py-1.5 text-xs transition
             hover:border-surface-600 hover:text-fg"
      onclick={onclose}
    >
      关闭 (Esc)
    </button>
  </header>

  <div class="relative z-10 min-h-0 flex-1">
    {#if failed}
      <div class="flex h-full items-center justify-center px-6">
        <p class="max-w-xl text-center text-sm text-rose-300">
          原图加载失败，文件可能已被移动或删除：<br />
          <span class="font-mono text-xs break-all text-rose-200/70">{artwork.originalPath}</span>
        </p>
      </div>
    {:else}
      <!-- 纯视觉交互面：真正的语义在父级的 role="dialog" 上 -->
      <div
        class="viewer__stage"
        role="presentation"
        bind:this={stage}
        onpointermove={onpointermove}
        onpointerleave={reset}
        style="--rx: {rx}deg; --ry: {ry}deg; --gx: {gx}%; --gy: {gy}%;"
      >
        <div class="viewer__scene">
          <div class="viewer__card">
            <!-- 光锥底：背光场 -->
            <div class="viewer__halo"></div>

            <!-- 主体。圆角裁切只能在这一层（见文件头注释） -->
            <div class="viewer__shot">
              <img
                src={source}
                alt={`${artwork.pid} p${artwork.page}`}
                draggable="false"
                class="viewer__img {loaded ? 'is-loaded' : ''}"
                onload={() => (loaded = true)}
                onerror={() => (failed = true)}
              />
            </div>

            <!-- 扫光（叶子节点，允许 mix-blend-mode） -->
            <div class="viewer__glare"></div>
            <div class="viewer__sheen"></div>

            <!-- 前景信息层 -->
            <div class="viewer__meta">
              <span class="viewer__chip">p{artwork.page}</span>
              <span class="viewer__path" data-selectable>{artwork.originalPath}</span>
            </div>
          </div>
        </div>

        {#if !loaded}
          <div class="viewer__spinner"></div>
        {/if}
      </div>
    {/if}
  </div>
</div>

<style lang="postcss">
  @reference "../../app.css";

  /* ---- 舞台：perspective 只放这里 ------------------------------------ */
  .viewer__stage {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    padding: 1rem 1rem 3rem;
    /* 大图约 60vh 高，1.5× 是「能明显看出景深但不变形」的档位 */
    perspective: 900px;
    perspective-origin: 50% 50%;
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

  .viewer__img {
    display: block;
    width: auto;
    height: auto;
    max-width: min(88vw, 1100px);
    max-height: 62vh;
    object-fit: contain;
    opacity: 0;
    transition: opacity 0.28s ease-out;
    backface-visibility: hidden;
  }

  .viewer__img.is-loaded {
    opacity: 1;
  }

  /* ---- 扫光：跟随鼠标的镜面高光（叶子） ------------------------------ */
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

  .viewer__chip {
    flex: none;
    border-radius: 0.25rem;
    background: color-mix(in oklab, var(--tone-accent) 22%, transparent);
    padding: 0.05rem 0.35rem;
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--tone-accent);
  }

  .viewer__path {
    min-width: 0;
    flex: 1;
    overflow: hidden;
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--tone-fg-soft);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* ---- 加载指示 ------------------------------------------------------ */
  .viewer__spinner {
    position: absolute;
    width: 2.5rem;
    height: 2.5rem;
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

<script lang="ts">
  /**
   * 筛选面板的外壳：**底部浮层与右侧抽屉共用同一份内容**。
   *
   * 为什么两种都留着：手机上筛选有两条主流路线，人机功效各有取舍 ——
   * 底部浮层只占下半屏、一眼还能看见上面的图，拇指也够得着；右侧抽屉更高、
   * 一屏放得下更多条件，但会挡住大半个列表。哪种更顺手因人而异，
   * 所以做成可切换、选择记在 localStorage 里。
   *
   * **动效是必须的**：上一版把带滑入动效的抽屉换成原地长出的面板时漏补了动效，
   * 面板变成「啪一下出现」，一眼就被看出来了。这里两种形态都带动效，
   * 收起时也能看到它滑回去，而不是凭空消失。
   *
   * 背幕与面板是兄弟节点：背幕负责「点外部关闭」，面板自己吃掉点击 ——
   * 合成一个节点的话，面板内部的点击会冒泡到背幕上，一点就把自己关了。
   */
  import type { Snippet } from "svelte";
  import { fade } from "svelte/transition";
  import { cubicOut } from "svelte/easing";

  import type { FilterForm } from "$lib/types";

  let {
    form,
    onclose,
    onform,
    children,
  }: {
    form: FilterForm;
    onclose: () => void;
    /** 切换形态，由调用方落盘（试人机功效用，不必重进页面）。 */
    onform: (next: FilterForm) => void;
    children: Snippet;
  } = $props();

  const FORMS: { value: FilterForm; label: string; title: string }[] = [
    { value: "sheet", label: "底部", title: "从底部升起，只占下半屏" },
    { value: "drawer", label: "侧边", title: "从右侧滑出，一屏能放更多条件" },
  ];

  /**
   * 滑入滑出用**自定义过渡**而不是 `fly`。
   *
   * `fly` 只收像素值，而浮层有多高取决于内容 —— 写死一个数要么滑不到位、
   * 要么第一帧就冲出屏幕。这里在进场时现场量一次自身尺寸，按它换算位移，
   * 退出时 Svelte 会复用同一组 `css`，所以两边都是「刚好滑满自己的高度」。
   */
  function slideUp(node: HTMLElement) {
    const span = node.offsetHeight || 480;
    return {
      duration: 280,
      easing: cubicOut,
      css: (t: number) => `transform: translateY(${(1 - t) * span}px);`,
    };
  }

  function slideRight(node: HTMLElement) {
    const span = node.offsetWidth || 380;
    return {
      duration: 280,
      easing: cubicOut,
      css: (t: number) => `transform: translateX(${(1 - t) * span}px);`,
    };
  }
</script>

<!-- 背幕：点它关闭。`md:hidden` 与面板一致 —— 两种形态都只服务窄屏，
     桌面走的是「维度入口 + 锚点浮层」，不需要遮罩 -->
<div
  class="fixed inset-0 z-40 bg-black/50 md:hidden"
  aria-hidden="true"
  transition:fade={{ duration: 180 }}
  onclick={onclose}
></div>

{#if form === "sheet"}
  <div
    class="fixed inset-x-0 bottom-0 z-50 flex max-h-[85vh] flex-col rounded-t-2xl border-t
           border-surface-800 floating-panel shadow-2xl md:hidden"
    role="dialog"
    aria-label="筛选作品"
    transition:slideUp
  >
    <!-- 抓手：一眼看出这是能往下拖走的东西（拖拽手势本身没做，点背幕/返回也能关） -->
    <div class="flex justify-center pt-2" aria-hidden="true">
      <div class="h-1 w-9 rounded-full bg-surface-700"></div>
    </div>

    <div class="flex items-center justify-between px-4 pt-1.5 pb-2">
      <span class="text-sm font-semibold text-fg">筛选</span>
      <div class="flex items-center gap-2">
        {@render formSwitch()}
        {@render closeButton()}
      </div>
    </div>

    <div
      class="min-h-0 flex-1 overflow-y-auto px-4 pb-[calc(1rem+env(safe-area-inset-bottom))]"
    >
      {@render children()}
    </div>
  </div>
{:else}
  <div
    class="fixed inset-y-0 right-0 z-50 flex w-[85%] max-w-96 flex-col border-l border-surface-800
           floating-panel shadow-2xl md:hidden"
    role="dialog"
    aria-label="筛选作品"
    transition:slideRight
  >
    <div
      class="flex items-center justify-between border-b border-surface-800 px-4 py-3
             pt-[calc(0.75rem+env(safe-area-inset-top))]"
    >
      <span class="text-sm font-semibold text-fg">筛选</span>
      <div class="flex items-center gap-2">
        {@render formSwitch()}
        {@render closeButton()}
      </div>
    </div>
    <div
      class="min-h-0 flex-1 overflow-y-auto px-4 py-3 pb-[calc(1rem+env(safe-area-inset-bottom))]"
    >
      {@render children()}
    </div>
  </div>
{/if}

{#snippet formSwitch()}
  <!-- 形态开关：就在筛选自己身上，试的时候不用跑去设置页来回找 -->
  <div class="flex items-center rounded-md border border-surface-700 p-0.5">
    {#each FORMS as option (option.value)}
      <button
        type="button"
        class="cursor-pointer rounded px-1.5 py-0.5 text-[11px] transition {form === option.value
          ? 'bg-accent-600 font-medium text-on-accent'
          : 'text-soft hover:text-fg'}"
        aria-pressed={form === option.value}
        title={option.title}
        onclick={() => onform(option.value)}
      >
        {option.label}
      </button>
    {/each}
  </div>
{/snippet}

{#snippet closeButton()}
  <button
    type="button"
    class="cursor-pointer rounded-md p-1.5 text-muted transition hover:bg-surface-850 hover:text-fg"
    aria-label="收起筛选"
    onclick={onclose}
  >
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-5">
      <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
    </svg>
  </button>
{/snippet}

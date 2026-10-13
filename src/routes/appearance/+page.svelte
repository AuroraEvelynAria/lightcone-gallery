<script lang="ts">
  /**
   * 外观：配色方案 × 窗口材质 × 明暗，三个正交的轴。
   *
   * 配色源自 RhinE（foobar2000 主题）的 5 套方案，另按主人反馈新增了 4 套；材质是 Tauri 的真实系统效果
   * （DWM Mica / Acrylic），不是 CSS 假模糊。切换即时生效，不需要重启。
   */
  import { goto } from "$app/navigation";

  import { registerBackCloser } from "$lib/backStack.svelte";
  import { theme } from "$lib/theme.svelte";
  import { findScheme, MATERIALS, SCHEMES, type SchemeId, type ThemeMode } from "$lib/themes";

  /** 手机上没有窗口材质这回事（WebView 背后没有 DWM），材质区整个收起。 */
  const isAndroid = navigator.userAgent.includes("Android");

  // Android 全面屏返回手势：外观是独立路由，按返回该退回主界面，而不是退出
  // 应用 —— 真机上一滑就回桌面，就是关闭栈里缺了「当前是子路由」这一层。
  $effect(() => registerBackCloser(() => void goto("/")));

  /**
   * 明暗三态。文案跟着平台走 —— 手机上写「跟着 Windows 的设置走」是错的，
   * 那边跟的是手机系统自己的浅色 / 深色开关。
   */
  const MODES: { id: ThemeMode; name: string; note: string }[] = [
    {
      id: "system",
      name: "跟随系统",
      note: isAndroid ? "跟着手机系统的浅色 / 深色设置走" : "跟着 Windows 的浅色 / 深色设置走",
    },
    { id: "light", name: "浅色", note: "固定浅色，忽略系统设置" },
    { id: "dark", name: "深色", note: "固定深色，忽略系统设置" },
  ];

  /** 取某方案在当前明暗下的代表色，用来画色条。 */
  function swatch(id: SchemeId): string[] {
    const palette = theme.dark ? findScheme(id).dark : findScheme(id).light;
    return [palette.bg, palette.panel, palette.case, palette.accent, palette.fg];
  }

  /** 选项外框的统一写法：选中时描边用强调色。 */
  function optionClass(active: boolean): string {
    return `cursor-pointer rounded-card border p-3 text-left transition ${
      active
        ? "border-accent-500 bg-accent-500/10"
        : "border-surface-800 hover:border-surface-600 hover:bg-surface-850/60"
    }`;
  }

  const preview = $derived(theme.palette);
</script>

<div class="flex h-screen flex-col bg-surface-950 text-soft">
  <!-- pt 补 safe-area：全面屏下这个独立路由的 header 会顶到状态栏底下 -->
  <header
    class="flex flex-wrap items-center gap-3 border-b border-surface-800 bg-surface-900 px-5 py-3
           pt-[calc(0.75rem+env(safe-area-inset-top))]"
  >
    <div class="min-w-0">
      <h1 class="text-sm font-semibold text-fg">外观</h1>
      <p class="text-[11px] text-muted">
        配色 <span class="font-mono text-accent-400">{findScheme(theme.scheme).name}</span>
        {#if !isAndroid}
          <!-- 材质是 Windows 专属，手机上报一个 solid/acrylic 只会让人困惑 -->
          · 材质 <span class="font-mono text-accent-400">{theme.material}</span>
        {/if}
        · {theme.dark ? "深色" : "浅色"}
      </p>
    </div>

    <div class="flex-1"></div>

    <a
      href="/"
      class="rounded-md border border-surface-700 px-3 py-1.5 text-xs text-soft transition hover:border-surface-600 hover:text-fg"
    >
      返回图库
    </a>
    <a
      href="/artworks"
      class="rounded-md border border-surface-700 px-3 py-1.5 text-xs text-soft transition hover:border-surface-600 hover:text-fg"
    >
      作品库
    </a>
  </header>

  {#if theme.error}
    <div class="border-b border-rose-900/60 bg-rose-950/60 px-5 py-2 text-xs text-rose-200">
      {theme.error}
    </div>
  {/if}

  <main class="min-h-0 flex-1 overflow-y-auto">
    <div class="mx-auto max-w-3xl space-y-8 px-6 py-8">
      <!-- 配色方案 -->
      <section>
        <div class="mb-3 flex items-baseline justify-between">
          <h2 class="font-mono text-[11px] font-semibold tracking-wide text-muted uppercase">
            配色方案
          </h2>
          <p class="text-[11px] text-mark">{SCHEMES.length} 套，各自带浅色与深色两版</p>
        </div>
        <div class="grid grid-cols-2 gap-3 sm:grid-cols-3">
          {#each SCHEMES as scheme (scheme.id)}
            {@const active = theme.scheme === scheme.id}
            <button
              type="button"
              class={optionClass(active)}
              aria-pressed={active}
              onclick={() => theme.setScheme(scheme.id)}
            >
              <div class="flex h-7 overflow-hidden rounded-md ring-1 ring-black/25">
                {#each swatch(scheme.id) as color, index (index)}
                  <span class="block flex-1" style="background: {color}"></span>
                {/each}
              </div>
              <div class="mt-2 flex items-center gap-1.5">
                {#if active}
                  <span class="size-1.5 rounded-full bg-accent-500"></span>
                {/if}
                <span class="font-mono text-[11px] font-semibold tracking-wide text-fg">
                  {scheme.name}
                </span>
              </div>
            </button>
          {/each}
        </div>
      </section>

      <!-- 窗口材质：手机端没有 DWM，整个收起（半透明令牌在 WebView 里会穿帮） -->
      {#if !isAndroid}
      <section>
        <div class="mb-3 flex items-baseline justify-between">
          <h2 class="font-mono text-[11px] font-semibold tracking-wide text-muted uppercase">
            窗口材质
          </h2>
          <p class="text-[11px] text-mark">系统级效果，窗口本身是透明的</p>
        </div>
        <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
          {#each MATERIALS as material (material.id)}
            {@const active = theme.material === material.id}
            <button
              type="button"
              class={optionClass(active)}
              aria-pressed={active}
              onclick={() => theme.setMaterial(material.id)}
            >
              <div class="flex items-baseline justify-between gap-2">
                <span class="text-sm font-medium text-fg">{material.name}</span>
                <span class="shrink-0 font-mono text-[10px] text-mark">{material.platform}</span>
              </div>
              <p class="mt-1 text-[11px] leading-relaxed text-muted">{material.note}</p>
            </button>
          {/each}
        </div>
        <p class="mt-3 text-[11px] leading-relaxed text-mark">
          材质由 DWM 绘制，只在不支持的系统上会静默失效 —— 底层库的返回值被 Tauri
          丢掉了，程序无法判断它有没有真的生效。选了没反应就说明当前系统不支持，
          换「纯色」即可。
        </p>
      </section>
      {/if}

      <!-- 明暗 -->
      <section>
        <div class="mb-3 flex items-baseline justify-between">
          <h2 class="font-mono text-[11px] font-semibold tracking-wide text-muted uppercase">
            明暗
          </h2>
          <p class="text-[11px] text-mark">
            {isAndroid ? "只影响配色" : "只影响配色，不影响材质本身的明暗"}
          </p>
        </div>
        <div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
          {#each MODES as mode (mode.id)}
            {@const active = theme.mode === mode.id}
            <button
              type="button"
              class={optionClass(active)}
              aria-pressed={active}
              onclick={() => theme.setMode(mode.id)}
            >
              <span class="text-sm font-medium text-fg">{mode.name}</span>
              <p class="mt-1 text-[11px] leading-relaxed text-muted">{mode.note}</p>
            </button>
          {/each}
        </div>
      </section>

      <!-- 预览 -->
      <section>
        <div class="mb-3 flex items-baseline justify-between">
          <h2 class="font-mono text-[11px] font-semibold tracking-wide text-muted uppercase">
            预览
          </h2>
          <p class="text-[11px] text-mark">当前配色的各层取色</p>
        </div>
        <div class="rounded-card border border-surface-800 bg-surface-900 p-4">
          <div class="flex flex-wrap items-center gap-2">
            <span class="rounded-md bg-accent-600 px-2.5 py-1 text-xs font-medium text-on-accent">
              强调按钮
            </span>
            <span class="rounded-md border border-surface-700 px-2.5 py-1 text-xs text-soft">
              次要按钮
            </span>
            <span class="rounded-md bg-accent-500/15 px-2.5 py-1 font-mono text-xs text-accent-400">
              123456
            </span>
          </div>

          <div class="mt-3 grid grid-cols-4 gap-2">
            {#each Array.from({ length: 4 }) as _, index (index)}
              <div
                class="aspect-square rounded-card border border-surface-800 bg-surface-850
                       ring-1 ring-white/5"
              ></div>
            {/each}
          </div>

          <div class="mt-3 rounded-md bg-well px-3 py-2 font-mono text-[11px] text-muted">
            well · 输入框 / 凹槽
          </div>

          <dl class="mt-3 grid grid-cols-2 gap-x-4 gap-y-1 font-mono text-[11px] sm:grid-cols-4">
            {#each Object.entries(preview) as [token, value] (token)}
              <div class="flex items-center gap-1.5">
                <span class="size-3 shrink-0 rounded-sm ring-1 ring-black/25" style="background: {value}"></span>
                <dt class="truncate text-mark">{token}</dt>
              </div>
            {/each}
          </dl>
        </div>
      </section>
    </div>
  </main>
</div>

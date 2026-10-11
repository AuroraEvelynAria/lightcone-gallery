/**
 * Android 全面屏返回手势的「关闭栈」。
 *
 * MainActivity 把系统返回转发成 `lc-android-back` 事件（见 +page.svelte 的监听），
 * 最上层注册者先处理：检视弹窗 → 侧栏抽屉 → 子浮层……栈空了才真的退出应用
 * （`exit_app` 命令）。桌面上没有这个事件，注册了也不会被触发，无副作用。
 *
 * 为什么要栈：返回的语义是「撤销最上面那层界面」，而浮层是随时叠加的 ——
 * 弹窗上面开抽屉、抽屉上面开确认框，各组件只管自己的那层，谁开着谁注册。
 */

type BackCloser = () => void;

const stack: BackCloser[] = [];

/**
 * 注册一个「按返回就执行」的关闭函数，返回注销函数。
 * 通常放在 `$effect` 里：`if (open) return registerBackCloser(...)`，
 * 组件销毁或浮层收起时自动注销。
 */
export function registerBackCloser(close: BackCloser): () => void {
  stack.push(close);
  return () => {
    const index = stack.indexOf(close);
    if (index >= 0) stack.splice(index, 1);
  };
}

/** 处理一次系统返回。有浮层就关最上层的并返回 true（已消费）。 */
export function handleBack(): boolean {
  const close = stack[stack.length - 1];
  if (!close) return false;
  close();
  return true;
}

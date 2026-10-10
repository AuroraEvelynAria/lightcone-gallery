/**
 * 缩略图请求闸门。
 *
 * 虚拟滚动会把整屏格子一次性推进视口。若不加限制，滚动到新区域时前端会同时向
 * 后端抛出上百个「解码整图 → 缩放 → 编码」任务 —— Rust 端每个任务都是
 * `spawn_blocking`，内存峰值能到 GB 级，低配机器会直接卡死。
 *
 * 这里用固定并发度的先进先出队列把在途请求压住：先到先得，滚动停下后逐张补齐。
 * 刻意不引依赖，整个实现就是「计数 + 等待队列」。
 */

const MAX_CONCURRENT = 4;

let active = 0;
const waiting: Array<() => void> = [];

async function acquire(): Promise<void> {
  // 用 while 而非 if：被唤醒后必须重新确认名额，避免多个唤醒者同时挤进去
  while (active >= MAX_CONCURRENT) {
    await new Promise<void>((resolve) => waiting.push(resolve));
  }
  active += 1;
}

function release(): void {
  active -= 1;
  waiting.shift()?.();
}

/** 排队执行一次缩略图请求；同屏大量请求会被压到 MAX_CONCURRENT 并发。 */
export async function withThumbnailSlot<T>(task: () => Promise<T>): Promise<T> {
  await acquire();
  try {
    return await task();
  } finally {
    release();
  }
}

import type { DecodeItemEvent, DecodeItemResult } from "./types";

export function decodeEventSlot(event: DecodeItemEvent, activeId: string, paths: string[], slots: number[]): number | undefined {
  // 任务编号、序号和路径共同校验，防止旧事件或单项重试把批次其他位置覆盖。
  // Validate task id, index, and path together so stale events and single-item retries cannot overwrite another batch slot.
  if (event.taskId !== activeId || !Number.isInteger(event.current) || event.current < 1) return undefined;
  const index = event.current - 1;
  if (paths[index] !== event.item.inputPath) return undefined;
  return slots[index];
}

export function decodeCountDelta(previous: DecodeItemResult | undefined, next: DecodeItemResult) {
  // 重复事件只改变状态差值，避免重发或最终快照让成功和完成计数持续增加。
  // Repeated events change only the status delta, keeping retransmissions and final snapshots from inflating counts.
  return { completed: previous ? 0 : 1, succeeded: Number(next.succeeded) - Number(previous?.succeeded ?? false) };
}

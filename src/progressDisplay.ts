export function clampFailurePercent(
  prev: number | undefined,
  incoming: number | undefined,
  outcome: string,
): number {
  if (outcome === "failed" || outcome === "cancelled") {
    return Math.round(Math.max(0, Math.min(prev ?? incoming ?? 0, 99)));
  }
  return Math.round(Math.max(0, Math.min(incoming ?? 0, 100)));
}

export function fileCounterLabel(p: { current: number; total: number; unitKind?: string }): string {
  if (p.unitKind === "frame" && p.total > 0) return "frame";
  return "file";
}

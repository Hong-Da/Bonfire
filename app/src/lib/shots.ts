import type { PurgeReport, ShotRecord } from "../types";

/**
 * 截图与存档快照的对应（与后端 `shot_snapshots` 相同）：拍摄之后最早的一个快照，
 * 即“这次存档前拍的截图”；拍摄后还没有快照的为 null。
 */
export function shotSnapshots(nodes: { id: string; createdAt: string }[], shots: ShotRecord[]): Map<string, string | null> {
  const snaps = nodes
    .map((n) => ({ id: n.id, t: Date.parse(n.createdAt) }))
    .filter((n) => !Number.isNaN(n.t))
    .sort((a, b) => a.t - b.t || a.id.localeCompare(b.id));
  const out = new Map<string, string | null>();
  for (const s of shots) {
    const t = Date.parse(s.takenAt);
    // 第一个时间不早于拍摄时间的快照（二分查找）。
    let lo = 0;
    let hi = snaps.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (snaps[mid].t < t) lo = mid + 1;
      else hi = mid;
    }
    out.set(s.blob, Number.isNaN(t) ? null : (snaps[lo]?.id ?? null));
  }
  return out;
}

/** 每个快照之前拍的截图数。 */
export function shotCounts(map: Map<string, string | null>): Map<string, number> {
  const counts = new Map<string, number>();
  for (const snap of map.values()) if (snap) counts.set(snap, (counts.get(snap) ?? 0) + 1);
  return counts;
}

/** 按拍摄日期（本地时间）分组，日期新的在前；组内按时间从新到旧。 */
export function groupByDay(shots: ShotRecord[]): { day: string; shots: ShotRecord[] }[] {
  const groups = new Map<string, ShotRecord[]>();
  const sorted = [...shots].sort((a, b) => Date.parse(b.takenAt) - Date.parse(a.takenAt));
  for (const s of sorted) {
    const d = new Date(s.takenAt);
    const key = Number.isNaN(d.getTime()) ? "" : `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
    groups.set(key, [...(groups.get(key) ?? []), s]);
  }
  return [...groups.entries()].map(([day, shots]) => ({ day, shots }));
}

/** 原路径中的文件名。 */
export function fileName(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

/** 彻底删除后的提示：删除数量，选了同时删除原图时再说明原图的处理结果。 */
export function purgeMessage(t: (k: string, o?: Record<string, unknown>) => string, reports: PurgeReport[], originals: boolean): string {
  const sum = (f: (r: PurgeReport) => number) => reports.reduce((n, r) => n + f(r), 0);
  let text = t("ui.shots.purged", { n: sum((r) => r.purged) });
  if (originals) text += t("ui.purge.originalsDone", { n: sum((r) => r.originalsDeleted), kept: sum((r) => r.originalsKept), failed: sum((r) => r.originalsFailed) });
  return text;
}

import { create } from "zustand";
import { api, errorMessage } from "./api";
import type { AppInfo, GameRow, GameState, Phase } from "./types";

export type View = { name: "library" } | { name: "game"; id: string } | { name: "settings" };
export type StateFilter = "all" | GameState;

export interface Toast {
  id: number;
  kind: "info" | "error" | "success";
  text: string;
}

interface Store {
  info: AppInfo | null;
  view: View;
  rows: Record<string, GameRow>;
  order: string[];
  phase: Phase;
  loading: boolean;
  selected: Set<string>;
  platformFilter: string;
  stateFilter: StateFilter;
  query: string;
  toasts: Toast[];
  /** 恢复后常驻的“撤销本次恢复”提示（按游戏）。 */
  undoable: Record<string, string>;

  setInfo: (info: AppInfo) => void;
  go: (view: View) => void;
  loadLibrary: (discover: boolean) => Promise<void>;
  upsertRow: (row: GameRow) => void;
  refreshRow: (id: string) => Promise<void>;
  toggle: (id: string) => void;
  selectAll: (ids: string[]) => void;
  invert: (ids: string[]) => void;
  clearSelection: () => void;
  setPlatformFilter: (p: string) => void;
  setStateFilter: (s: StateFilter) => void;
  setQuery: (q: string) => void;
  toast: (kind: Toast["kind"], text: string) => void;
  dismiss: (id: number) => void;
  setUndoable: (gameId: string, target: string | null) => void;
}

let toastSeq = 0;

/** 默认勾选“有更新”和“未备份”的游戏（文档 8.1 第 1 步）。 */
export function defaultSelection(rows: GameRow[]): Set<string> {
  return new Set(rows.filter((r) => r.state !== "up-to-date").map((r) => r.id));
}

export function visibleRows(s: Pick<Store, "rows" | "order" | "platformFilter" | "stateFilter" | "query">): GameRow[] {
  const q = s.query.trim().toLowerCase();
  return s.order
    .map((id) => s.rows[id])
    .filter((r): r is GameRow => !!r)
    .filter((r) => s.platformFilter === "all" || r.platform === s.platformFilter)
    .filter((r) => s.stateFilter === "all" || r.state === s.stateFilter)
    .filter((r) => !q || r.name.toLowerCase().includes(q) || (r.nameZh ?? "").toLowerCase().includes(q) || r.id.includes(q));
}

/** 排序：有更新 → 未备份 → 已是最新，同组按名称。 */
function sortIds(rows: Record<string, GameRow>): string[] {
  const rank: Record<GameState, number> = { "has-changes": 0, "never-backed-up": 1, "up-to-date": 2 };
  return Object.values(rows)
    .sort((a, b) => rank[a.state] - rank[b.state] || (a.nameZh ?? a.name).localeCompare(b.nameZh ?? b.name))
    .map((r) => r.id);
}

export const useStore = create<Store>((set, get) => ({
  info: null,
  view: { name: "library" },
  rows: {},
  order: [],
  phase: { phase: "idle", done: 0, total: 0 },
  loading: false,
  selected: new Set(),
  platformFilter: "all",
  stateFilter: "all",
  query: "",
  toasts: [],
  undoable: {},

  setInfo: (info) => set({ info }),
  go: (view) => set({ view }),

  loadLibrary: async (discover) => {
    if (get().loading) return;
    set({ loading: true });
    const unlistenRow = await api.onLibraryRow((row) => get().upsertRow(row));
    const unlistenPhase = await api.onLibraryPhase((phase) => set({ phase }));
    try {
      const rows = await api.loadLibrary(discover);
      const map = Object.fromEntries(rows.map((r) => [r.id, r]));
      // 用户在加载过程中已经勾选过时保留其选择；否则默认勾选有更新与未备份的游戏。
      const keep = get().selected.size > 0 && [...get().selected].every((id) => map[id]);
      set({ rows: map, order: sortIds(map), selected: keep ? get().selected : defaultSelection(rows) });
    } catch (e) {
      get().toast("error", errorMessage(e));
    } finally {
      unlistenRow();
      unlistenPhase();
      set({ loading: false, phase: { phase: "idle", done: 0, total: 0 } });
    }
  },

  upsertRow: (row) =>
    set((s) => {
      const rows = { ...s.rows, [row.id]: row };
      const order = s.order.includes(row.id) ? s.order : [...s.order, row.id];
      return { rows, order };
    }),

  refreshRow: async (id) => {
    try {
      const row = await api.gameRow(id);
      set((s) => {
        const rows = { ...s.rows, [id]: row };
        return { rows, order: sortIds(rows) };
      });
    } catch (e) {
      get().toast("error", errorMessage(e));
    }
  },

  toggle: (id) =>
    set((s) => {
      const selected = new Set(s.selected);
      if (selected.has(id)) selected.delete(id);
      else selected.add(id);
      return { selected };
    }),
  selectAll: (ids) => set((s) => ({ selected: new Set([...s.selected, ...ids]) })),
  invert: (ids) =>
    set((s) => {
      const selected = new Set(s.selected);
      for (const id of ids) {
        if (selected.has(id)) selected.delete(id);
        else selected.add(id);
      }
      return { selected };
    }),
  clearSelection: () => set({ selected: new Set() }),
  setPlatformFilter: (platformFilter) => set({ platformFilter }),
  setStateFilter: (stateFilter) => set({ stateFilter }),
  setQuery: (query) => set({ query }),

  toast: (kind, text) => {
    const id = ++toastSeq;
    set((s) => ({ toasts: [...s.toasts, { id, kind, text }] }));
    if (kind !== "error") setTimeout(() => get().dismiss(id), 4000);
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
  setUndoable: (gameId, target) =>
    set((s) => {
      const undoable = { ...s.undoable };
      if (target) undoable[gameId] = target;
      else delete undoable[gameId];
      return { undoable };
    }),
}));

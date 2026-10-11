import { create } from "zustand";
import { api, errorMessage } from "./api";
import type { AppInfo, GameRow, GameState, ManageMode, Phase, ScanReport } from "./types";
import { fuzzyMatch } from "./lib/search";

export type View = { name: "library" } | { name: "gallery" } | { name: "scan" } | { name: "game"; id: string } | { name: "settings" };
export type StateFilter = "all" | GameState;
export type ModeFilter = "all" | ManageMode;

/** 启动时的页面：地址中的 `#/gallery`、`#/scan`、`#/settings`、`#/game/<id>`，没有时为游戏库。 */
export function initialView(hash: string): View {
  const [name, id] = hash.replace(/^#\/?/, "").split("/");
  if (name === "gallery" || name === "scan" || name === "settings") return { name };
  if (name === "game" && id) return { name: "game", id: decodeURIComponent(id) };
  return { name: "library" };
}

export interface Toast {
  id: number;
  kind: "info" | "error" | "success";
  text: string;
  /** 提示中的操作按钮（如“撤销”）。 */
  action?: { label: string; run: () => void };
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
  modeFilter: ModeFilter;
  /** 当前使用的筛选方案：只列出方案中的游戏。 */
  profileFilter: { name: string; ids: string[] } | null;
  query: string;
  toasts: Toast[];
  /** 恢复后常驻的“撤销本次恢复”提示（按游戏）。 */
  undoable: Record<string, string>;
  /** 最近一次扫描（发现游戏）的结果，离开扫描页后保留。 */
  scan: ScanReport | null;
  scanning: boolean;
  scanPhase: "rules" | "discovering" | "idle";

  setInfo: (info: AppInfo) => void;
  go: (view: View) => void;
  loadLibrary: (fetchInfo: boolean) => Promise<void>;
  startScan: () => Promise<void>;
  upsertRow: (row: GameRow) => void;
  refreshRow: (id: string) => Promise<void>;
  toggle: (id: string) => void;
  selectAll: (ids: string[]) => void;
  invert: (ids: string[]) => void;
  clearSelection: () => void;
  setPlatformFilter: (p: string) => void;
  setStateFilter: (s: StateFilter) => void;
  setModeFilter: (m: ModeFilter) => void;
  setProfileFilter: (p: { name: string; ids: string[] } | null) => void;
  setQuery: (q: string) => void;
  toast: (kind: Toast["kind"], text: string, action?: Toast["action"]) => void;
  dismiss: (id: number) => void;
  setUndoable: (gameId: string, target: string | null) => void;
}

let toastSeq = 0;

/** 默认勾选“有更新”和“未备份”的游戏（文档 8.1 第 1 步）。 */
export function defaultSelection(rows: GameRow[]): Set<string> {
  return new Set(rows.filter((r) => r.state !== "up-to-date").map((r) => r.id));
}

export function visibleRows(
  s: Pick<Store, "rows" | "order" | "platformFilter" | "stateFilter" | "query"> & { modeFilter?: ModeFilter; profileFilter?: Store["profileFilter"] },
): GameRow[] {
  const q = s.query.trim().toLowerCase();
  return s.order
    .map((id) => s.rows[id])
    .filter((r): r is GameRow => !!r)
    .filter((r) => s.platformFilter === "all" || r.platform === s.platformFilter)
    .filter((r) => s.stateFilter === "all" || r.state === s.stateFilter)
    .filter((r) => !s.modeFilter || s.modeFilter === "all" || r.mode === s.modeFilter)
    .filter((r) => !s.profileFilter || s.profileFilter.ids.includes(r.id))
    // 模糊搜索：中文名、英文名、拼音（首字母）、按顺序出现的字母；游戏 ID 按包含匹配。
    .filter((r) => !q || r.id.includes(q) || fuzzyMatch(q, [r.name, r.nameZh]));
}

/** 排序：有更新 → 未备份 → 已是最新 → 只管理截图，同组按名称。 */
function sortIds(rows: Record<string, GameRow>): string[] {
  const rank: Record<GameState, number> = { "has-changes": 0, "never-backed-up": 1, "up-to-date": 2, "shots-only": 3 };
  return Object.values(rows)
    .sort((a, b) => rank[a.state] - rank[b.state] || (a.nameZh ?? a.name).localeCompare(b.nameZh ?? b.name))
    .map((r) => r.id);
}

export const useStore = create<Store>((set, get) => ({
  info: null,
  view: initialView(typeof location === "undefined" ? "" : location.hash),
  rows: {},
  order: [],
  phase: { phase: "idle", done: 0, total: 0 },
  loading: false,
  selected: new Set(),
  platformFilter: "all",
  stateFilter: "all",
  modeFilter: "all",
  profileFilter: null,
  query: "",
  toasts: [],
  undoable: {},
  scan: null,
  scanning: false,
  scanPhase: "idle",

  setInfo: (info) => set({ info }),
  go: (view) => set({ view }),

  startScan: async () => {
    if (get().scanning) return;
    set({ scanning: true, scanPhase: "rules" });
    const unlisten = await api.onScanPhase((scanPhase) => set({ scanPhase }));
    try {
      set({ scan: await api.discoverGames() });
    } catch (e) {
      get().toast("error", errorMessage(e));
    } finally {
      unlisten();
      set({ scanning: false, scanPhase: "idle" });
    }
  },

  loadLibrary: async (fetchInfo) => {
    if (get().loading) return;
    set({ loading: true });
    const unlistenRow = await api.onLibraryRow((row) => get().upsertRow(row));
    const unlistenPhase = await api.onLibraryPhase((phase) => set({ phase }));
    try {
      const rows = await api.loadLibrary(fetchInfo);
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
  setModeFilter: (modeFilter) => set({ modeFilter }),
  setProfileFilter: (profileFilter) => set({ profileFilter }),
  setQuery: (query) => set({ query }),

  toast: (kind, text, action) => {
    const id = ++toastSeq;
    set((s) => ({ toasts: [...s.toasts, { id, kind, text, action }] }));
    // 带操作的提示多留一会儿，便于点“撤销”。
    if (kind !== "error") setTimeout(() => get().dismiss(id), action ? 8000 : 4000);
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

import { create } from "zustand";
import { prefGet, prefRemove, prefSet } from "./lib/uiState";
import { markSeen, noteGames } from "./lib/fresh";
import { api, errorMessage } from "./api";
import type { AppInfo, GameRow, GameState, ManageMode, Phase, ScanReport } from "./types";
import { fuzzyMatch } from "./lib/search";

/** `section`：打开设置时跳到的分组（如快捷菜单中的“更多设置”）。 */
export type View = { name: "library" } | { name: "gallery" } | { name: "scan" } | { name: "game"; id: string } | { name: "settings"; section?: "auto" | "shots" };
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
  /** 只看新加入的游戏 */
  newOnly: boolean;
  /** 新加入的游戏：ID → 加入时间（见 lib/fresh.ts） */
  fresh: Record<string, number>;
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
  setNewOnly: (on: boolean) => void;
  /** 启动读入界面状态后恢复上次的筛选 */
  restoreFilters: () => void;
  /** 标为已看过（不再显示“新加入”） */
  markSeen: (ids: string[]) => void;
  setQuery: (q: string) => void;
  toast: (kind: Toast["kind"], text: string, action?: Toast["action"]) => void;
  dismiss: (id: number) => void;
  setUndoable: (gameId: string, target: string | null) => void;
}

let toastSeq = 0;

/** 游戏库的筛选（状态、管理方式、平台、筛选方案）记在本机，重新打开时恢复。 */
const FILTER_KEY = "bonfire.library.filters";
type SavedFilters = { stateFilter: StateFilter; modeFilter: ModeFilter; platformFilter: string; profileFilter: { name: string; ids: string[] } | null; newOnly: boolean };
const STATE_VALUES: StateFilter[] = ["all", "has-changes", "never-backed-up", "up-to-date", "shots-only"];
const MODE_VALUES: ModeFilter[] = ["all", "both", "saves", "shots"];

export function readFilters(): SavedFilters {
  const none: SavedFilters = { stateFilter: "all", modeFilter: "all", platformFilter: "all", profileFilter: null, newOnly: false };
  try {
    const v = JSON.parse(prefGet(FILTER_KEY) ?? "{}");
    const p = v.profileFilter;
    return {
      stateFilter: STATE_VALUES.includes(v.stateFilter) ? v.stateFilter : "all",
      modeFilter: MODE_VALUES.includes(v.modeFilter) ? v.modeFilter : "all",
      platformFilter: typeof v.platformFilter === "string" && v.platformFilter ? v.platformFilter : "all",
      profileFilter: p && typeof p.name === "string" && Array.isArray(p.ids) ? { name: p.name, ids: p.ids.filter((x: unknown) => typeof x === "string") } : null,
      newOnly: v.newOnly === true,
    };
  } catch {
    return none;
  }
}

function saveFilters(s: SavedFilters) {
  try {
    prefSet(FILTER_KEY, JSON.stringify(s));
  } catch {
    // 无法保存时只影响本次。
  }
}

const savedFilters = readFilters();

/** 游戏库中勾选的游戏也记住（U-39）：重新打开时恢复；从没保存过时按默认勾选有更新与未备份的游戏。 */
const SELECTION_KEY = "bonfire.library.selected";

export function readSelection(): string[] | null {
  try {
    const v = JSON.parse(prefGet(SELECTION_KEY) ?? "null");
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : null;
  } catch {
    return null;
  }
}

/** 默认勾选“有更新”和“未备份”的游戏（文档 8.1 第 1 步）。 */
export function defaultSelection(rows: GameRow[]): Set<string> {
  return new Set(rows.filter((r) => r.state !== "up-to-date").map((r) => r.id));
}

export function visibleRows(
  s: Pick<Store, "rows" | "order" | "platformFilter" | "stateFilter" | "query"> & {
    modeFilter?: ModeFilter;
    profileFilter?: Store["profileFilter"];
    newOnly?: boolean;
    fresh?: Record<string, number>;
  },
): GameRow[] {
  const q = s.query.trim().toLowerCase();
  return s.order
    .map((id) => s.rows[id])
    .filter((r): r is GameRow => !!r)
    .filter((r) => s.platformFilter === "all" || r.platform === s.platformFilter)
    .filter((r) => s.stateFilter === "all" || r.state === s.stateFilter)
    .filter((r) => !s.modeFilter || s.modeFilter === "all" || r.mode === s.modeFilter)
    .filter((r) => !s.profileFilter || s.profileFilter.ids.includes(r.id))
    .filter((r) => !s.newOnly || !!s.fresh?.[r.id])
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
  platformFilter: savedFilters?.platformFilter ?? "all",
  stateFilter: savedFilters?.stateFilter ?? "all",
  modeFilter: savedFilters?.modeFilter ?? "all",
  profileFilter: savedFilters?.profileFilter ?? null,
  newOnly: savedFilters?.newOnly ?? false,
  fresh: {},
  query: "",
  toasts: [],
  undoable: {},
  scan: null,
  scanning: false,
  scanPhase: "idle",

  setInfo: (info) => {
    // 关闭“记住”时清除已保存的选择，之后也不再保存。
    if (info.rememberFilters === false) prefRemove(FILTER_KEY);
    if (info.rememberSelection === false) prefRemove(SELECTION_KEY);
    set({ info });
  },
  go: (view) => {
    // 打开游戏详情后不再标为“新加入”。
    if (view.name === "game" && get().fresh[view.id]) get().markSeen([view.id]);
    set({ view });
  },

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
      const saved = get().info?.rememberSelection === false ? null : readSelection();
      const selected = keep ? get().selected : saved ? new Set(saved.filter((id) => map[id])) : defaultSelection(rows);
      set({ rows: map, order: sortIds(map), selected, fresh: noteGames(rows.map((r) => r.id)) });
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
  setPlatformFilter: (platformFilter) => {
    set({ platformFilter });
    persistFilters(get());
  },
  setStateFilter: (stateFilter) => {
    set({ stateFilter });
    persistFilters(get());
  },
  setModeFilter: (modeFilter) => {
    set({ modeFilter });
    persistFilters(get());
  },
  setProfileFilter: (profileFilter) => {
    set({ profileFilter });
    persistFilters(get());
  },
  restoreFilters: () => set(get().info?.rememberFilters === false ? DEFAULT_FILTERS : readFilters()),
  setNewOnly: (newOnly) => {
    set({ newOnly });
    persistFilters(get());
  },
  markSeen: (ids) => {
    markSeen(ids);
    set((s) => {
      const fresh = { ...s.fresh };
      for (const id of ids) delete fresh[id];
      return { fresh, newOnly: Object.keys(fresh).length > 0 && s.newOnly };
    });
    persistFilters(get());
  },
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

const DEFAULT_FILTERS: SavedFilters = { stateFilter: "all", modeFilter: "all", platformFilter: "all", profileFilter: null, newOnly: false };

function persistFilters(s: SavedFilters & { info?: Store["info"] }) {
  if (s.info?.rememberFilters === false) return;
  saveFilters({ stateFilter: s.stateFilter, modeFilter: s.modeFilter, platformFilter: s.platformFilter, profileFilter: s.profileFilter, newOnly: s.newOnly });
}

// 勾选变化时保存（加载游戏库之前的空勾选不保存）。
useStore.subscribe((s, prev) => {
  if (s.selected !== prev.selected && Object.keys(s.rows).length > 0 && s.info?.rememberSelection !== false)
    prefSet(SELECTION_KEY, JSON.stringify([...s.selected]));
});

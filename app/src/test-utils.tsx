import { vi } from "vitest";
import type { GameRow, HistoryView } from "./types";

/** 模拟的后端：每个测试文件用 vi.mock("./api") 引用这里的实现。 */
export function mockApi() {
  return {
    appInfo: vi.fn(),
    getSettings: vi.fn(),
    saveSettings: vi.fn(),
    loadLibrary: vi.fn().mockResolvedValue([]),
    discoverGames: vi.fn(),
    includeGames: vi.fn().mockResolvedValue([]),
    ignoreGames: vi.fn().mockResolvedValue([]),
    onScanPhase: vi.fn().mockResolvedValue(() => {}),
    gameRow: vi.fn(),
    gameChanges: vi.fn().mockResolvedValue([]),
    cancelTask: vi.fn(),
    runBackup: vi.fn(),
    history: vi.fn(),
    snapshotFiles: vi.fn().mockResolvedValue([]),
    diff: vi.fn().mockResolvedValue([]),
    setComment: vi.fn(),
    createRef: vi.fn(),
    searchComments: vi.fn().mockResolvedValue([]),
    planRestore: vi.fn(),
    restore: vi.fn(),
    undoRestore: vi.fn(),
    pendingUndo: vi.fn().mockResolvedValue(null),
    addGame: vi.fn(),
    removeGame: vi.fn(),
    confirmLocations: vi.fn(),
    ignoredGames: vi.fn().mockResolvedValue([]),
    enableGame: vi.fn(),
    gameInfo: vi.fn().mockResolvedValue(null),
    refreshInfo: vi.fn(),
    setInfoField: vi.fn(),
    profiles: vi.fn().mockResolvedValue([]),
    saveProfile: vi.fn(),
    deleteProfile: vi.fn(),
    fsck: vi.fn(),
    reindex: vi.fn(),
    updateRules: vi.fn(),
    rulesInfo: vi.fn().mockResolvedValue({ path: "x", games: 23606, userGames: 0 }),
    openPath: vi.fn(),
    onLibraryRow: vi.fn().mockResolvedValue(() => {}),
    onLibraryPhase: vi.fn().mockResolvedValue(() => {}),
    onBackupProgress: vi.fn().mockResolvedValue(() => {}),
  };
}

/** 所有测试文件共用的模拟后端实例。 */
export const sharedApi = mockApi();

export function row(over: Partial<GameRow> = {}): GameRow {
  return {
    id: "steam-367520",
    name: "Hollow Knight",
    nameZh: "空洞骑士",
    platform: "steam",
    enabled: true,
    coverThumb: null,
    state: "has-changes",
    summary: { added: 0, modified: 2, deleted: 0, unchanged: 3, changedBytes: 2048 },
    files: 5,
    totalBytes: 10240,
    lastModifiedNs: null,
    head: "b3:aaaaaaaa",
    branch: "main",
    detached: false,
    snapshots: 2,
    lastBackupAt: "2026-10-01T10:00:00+08:00",
    cloudSave: false,
    playtimeMin: 2520,
    unconfirmed: 0,
    ...over,
  };
}

export function history(): HistoryView {
  return {
    game: { id: "steam-367520", name: "Hollow Knight", platform: "steam", enabled: true },
    head: "b3:cccccccc",
    branch: "main",
    detached: false,
    refs: [
      { name: "main", kind: "branch", target: "b3:cccccccc", current: true },
      { name: "Boss 前", kind: "tag", target: "b3:aaaaaaaa", current: false },
    ],
    nodes: [
      { id: "b3:cccccccc", parents: ["b3:bbbbbbbb"], createdAt: "2026-10-09T10:00:00+08:00", trigger: "manual", comment: "白宫之前", files: 2, bytes: 100, changed: 1, playtimeMin: 3000, host: "PC", refs: ["heads/main"] },
      { id: "b3:bbbbbbbb", parents: ["b3:aaaaaaaa"], createdAt: "2026-10-08T10:00:00+08:00", trigger: "pre-restore", comment: "恢复前的自动保护", files: 2, bytes: 100, changed: 1, playtimeMin: null, host: "PC", refs: [] },
      { id: "b3:aaaaaaaa", parents: [], createdAt: "2026-10-07T10:00:00+08:00", trigger: "manual", comment: "打完螳螂领主\n第二行", files: 2, bytes: 100, changed: 2, playtimeMin: 2520, host: "PC", refs: ["tags/Boss 前"] },
    ],
    locations: [{ pathTemplate: "x", resolvedPath: "C:/Users/p/AppData/LocalLow/Team Cherry/Hollow Knight/*.dat", kind: "glob", tag: "save", source: "manifest", confirmed: true }],
  };
}

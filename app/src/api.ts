import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AlbumView,
  AutoReport,
  AutoStatus,
  GalleryView,
  ManageMode,
  PurgeReport,
  ShotDiff,
  WriteBackReport,
  ShotCandidate,
  ShotExportReport,
  ShotSyncReport,
  ArchiveImportItem,
  ArchivePreview,
  CollectionReport,
  DeleteReport,
  FilesAutoReport,
  FilesSource,
  BundleImportReport,
  BundlePreview,
  BundleReport,
  BundleScope,
  ExportFormat,
  ExportReport,
  FilesPreview,
  ImportFilesReport,
  ImportPlace,
  AppInfo,
  BackupItemResult,
  CmdError,
  Config,
  DiffEntry,
  FileChange,
  FsckReport,
  GameRecord,
  GameRow,
  HistoryView,
  MetadataView,
  Phase,
  Profile,
  RestorePlan,
  RestoreReport,
  ScanReport,
  SearchHit,
  TreeEntry,
} from "./types";

/** 前端只通过这里调用后端；命令名与 Tauri 命令层一一对应。 */

export function errorMessage(e: unknown): string {
  if (typeof e === "object" && e && "message" in e) return String((e as CmdError).message);
  return String(e);
}

export function errorKind(e: unknown): string {
  if (typeof e === "object" && e && "kind" in e) return String((e as CmdError).kind);
  return "general";
}

/** 本地缓存图片 → WebView 可加载的地址。 */
export function imageSrc(path: string | null | undefined): string | undefined {
  return path ? convertFileSrc(path) : undefined;
}

export type OnDetached = { mode: "new" } | { mode: "named"; name: string } | { mode: "move" };

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  getSettings: () => invoke<Config>("get_settings"),
  saveSettings: (config: Config) => invoke<AppInfo>("save_settings", { config }),

  loadLibrary: (fetchInfo: boolean, forceVerify = false) => invoke<GameRow[]>("load_library", { fetchInfo, forceVerify }),
  discoverGames: () => invoke<ScanReport>("discover_games"),
  includeGames: (ids: string[]) => invoke<GameRecord[]>("include_games", { ids }),
  ignoreGames: (ids: string[]) => invoke<GameRecord[]>("ignore_games", { ids }),
  gameRow: (gameId: string) => invoke<GameRow>("game_row", { gameId }),
  gameChanges: (gameId: string) => invoke<FileChange[]>("game_changes", { gameId }),
  cancelTask: () => invoke<void>("cancel_task"),

  runBackup: (items: { gameId: string; comment?: string; tag?: string }[], force: boolean, onDetached: OnDetached) =>
    invoke<BackupItemResult[]>("run_backup", { items, force, onDetached }),

  history: (gameId: string) => invoke<HistoryView>("history", { gameId }),
  snapshotFiles: (gameId: string, snapshot: string) => invoke<TreeEntry[]>("snapshot_files", { gameId, snapshot }),
  diff: (gameId: string, a: string, b: string | null) => invoke<DiffEntry[]>("diff", { gameId, a, b }),
  setComment: (gameId: string, snapshot: string, comment: string) => invoke<void>("set_comment", { gameId, snapshot, comment }),
  createRef: (gameId: string, kind: "branch" | "tag", name: string, snapshot: string | null) =>
    invoke<void>("create_ref", { gameId, kind, name, snapshot }),
  searchComments: (query: string) => invoke<SearchHit[]>("search_comments", { query }),

  planRestore: (gameId: string, target: string, paths: string[], full: boolean) =>
    invoke<RestorePlan>("plan_restore", { gameId, target, paths, full }),
  restore: (gameId: string, target: string, paths: string[], full: boolean, force: boolean) =>
    invoke<RestoreReport>("restore", { gameId, target, paths, full, force }),
  undoRestore: (gameId: string) => invoke<RestoreReport>("undo_restore", { gameId }),
  pendingUndo: (gameId: string) => invoke<{ target: string; protection: string | null } | null>("pending_undo", { gameId }),

  addGame: (name: string, savePaths: string[], configPaths: string[], installDir: string | null) =>
    invoke<GameRecord>("add_game", { name, savePaths, configPaths, installDir }),
  removeGame: (gameId: string) => invoke<GameRecord>("remove_game", { gameId }),
  confirmLocations: (gameId: string) => invoke<number>("confirm_locations", { gameId }),
  ignoredGames: () => invoke<GameRecord[]>("ignored_games"),
  enableGame: (gameId: string) => invoke<GameRecord>("enable_game", { gameId }),
  gameInfo: (gameId: string) => invoke<MetadataView | null>("game_info", { gameId }),
  refreshInfo: (gameId: string) => invoke<void>("refresh_info", { gameId }),
  setInfoField: (gameId: string, field: string, value: string) => invoke<void>("set_info_field", { gameId, field, value }),

  profiles: () => invoke<Profile[]>("profiles"),
  saveProfile: (profile: Profile) => invoke<void>("save_profile", { profile }),
  deleteProfile: (name: string) => invoke<void>("delete_profile", { name }),

  fsck: (deep: boolean) => invoke<FsckReport>("fsck", { deep }),
  reindex: () => invoke<{ snapshots: number; refs: number }>("reindex"),
  updateRules: () => invoke<{ result: "updated"; games: number; bytes: number } | { result: "notModified" }>("update_rules"),
  rulesInfo: () => invoke<{ path: string | null; games: number; userGames: number }>("rules_info"),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  openLocation: (gameId: string, ord: number) => invoke<void>("open_location", { gameId, ord }),

  exportDefaultName: (gameId: string, spec: string | null) => invoke<string>("export_default_name", { gameId, spec }),
  exportFiles: (gameId: string, spec: string | null, dest: string, format: ExportFormat) => invoke<ExportReport>("export_files", { gameId, spec, dest, format }),
  exportBundle: (gameId: string, scope: BundleScope, dest: string, includeShots: boolean) => invoke<BundleReport>("export_bundle", { gameId, scope, dest, includeShots }),
  inspectBundle: (path: string, gameId: string | null) => invoke<BundlePreview>("inspect_bundle", { path, gameId }),
  importBundle: (path: string, gameId: string | null) => invoke<BundleImportReport>("import_bundle", { path, gameId }),
  inspectFiles: (gameId: string, src: string) => invoke<FilesPreview>("inspect_files", { gameId, src }),
  importFiles: (gameId: string, src: string, place: ImportPlace, comment: string | null) => invoke<ImportFilesReport>("import_files", { gameId, src, place, comment }),
  exportCollection: (gameIds: string[], dest: string, includeShots: boolean) => invoke<CollectionReport>("export_collection", { gameIds, dest, includeShots }),
  album: (gameId: string) => invoke<AlbumView>("album", { gameId }),
  syncShots: (gameId: string | null) => invoke<ShotSyncReport[]>("sync_shots", { gameId }),
  shotThumb: (gameId: string, blob: string) => invoke<string>("shot_thumb", { gameId, blob }),
  shotImage: (gameId: string, blob: string) => invoke<string>("shot_image", { gameId, blob }),
  deleteShots: (gameId: string, blobs: string[]) => invoke<number>("delete_shots", { gameId, blobs }),
  exportShots: (gameId: string, blobs: string[], dest: string, format: ExportFormat) => invoke<ShotExportReport>("export_shots", { gameId, blobs, dest, format }),
  setShotsEnabled: (gameId: string, enabled: boolean | null) => invoke<void>("set_shots_enabled", { gameId, enabled }),
  addShotDir: (gameId: string, dir: string) => invoke<string>("add_shot_dir", { gameId, dir }),
  removeShotDir: (gameId: string, dir: string) => invoke<boolean>("remove_shot_dir", { gameId, dir }),
  capturesDir: () => invoke<string | null>("captures_dir"),
  restoreShots: (gameId: string, blobs: string[]) => invoke<number>("restore_shots", { gameId, blobs }),
  /** 彻底删除；`blobs` 为空时清空回收站。 */
  purgeShots: (gameId: string, blobs: string[], originals = false) => invoke<PurgeReport>("purge_shots", { gameId, blobs, originals }),
  setShotsOnly: (gameId: string, on: boolean) => invoke<void>("set_shots_only", { gameId, on }),
  shotCandidates: () => invoke<ShotCandidate[]>("shot_candidates"),
  addShotsGame: (key: string | null, name: string, dirs: string[]) => invoke<GameRecord>("add_shots_game", { key, name, dirs }),
  autoStatus: () => invoke<AutoStatus>("auto_status"),
  setManageMode: (gameIds: string[], mode: ManageMode) => invoke<void>("set_manage_mode", { gameIds, mode }),
  gallery: () => invoke<GalleryView>("gallery"),
  shotDiff: (gameId: string) => invoke<ShotDiff>("shot_diff", { gameId }),
  writeBackShots: (gameId: string, blobs: string[], dest: string | null) => invoke<WriteBackReport>("write_back_shots", { gameId, blobs, dest }),
  recollectShots: (gameId: string, paths: string[]) => invoke<number>("recollect_shots", { gameId, paths }),
  runningGames: (gameIds: string[]) => invoke<{ gameId: string; processes: string[] }[]>("running_games", { gameIds }),
  closeWindow: (tray: boolean, remember: boolean) => invoke<void>("close_window", { tray, remember }),
  onCloseRequested: (cb: () => void): Promise<UnlistenFn> => listen("close-requested", () => cb()),
  onAutoBackup: (cb: (r: AutoReport) => void): Promise<UnlistenFn> => listen<AutoReport>("auto-backup", (e) => cb(e.payload)),
  onShotsSynced: (cb: (added: number) => void): Promise<UnlistenFn> => listen<number>("shots-synced", (e) => cb(e.payload)),
  inspectArchive: (path: string) => invoke<ArchivePreview>("inspect_archive", { path }),
  importArchive: (path: string, picks: { key: string; target: string | null }[]) => invoke<ArchiveImportItem[]>("import_archive", { path, picks }),
  inspectFilesSource: (src: string) => invoke<FilesSource>("inspect_files_source", { src }),
  importFilesAuto: (src: string, comment: string | null) => invoke<FilesAutoReport>("import_files_auto", { src, comment }),
  deleteBranch: (gameId: string, name: string, dryRun: boolean) => invoke<DeleteReport>("delete_branch", { gameId, name, dryRun }),
  deleteTag: (gameId: string, name: string) => invoke<void>("delete_tag", { gameId, name }),
  deleteSnapshot: (gameId: string, snapshot: string, dryRun: boolean) => invoke<DeleteReport>("delete_snapshot", { gameId, snapshot, dryRun }),

  onTrayCheck: (cb: () => void): Promise<UnlistenFn> => listen("tray-check", () => cb()),
  onLibraryRow: (cb: (row: GameRow) => void): Promise<UnlistenFn> => listen<GameRow>("library-row", (e) => cb(e.payload)),
  onLibraryPhase: (cb: (p: Phase) => void): Promise<UnlistenFn> => listen<Phase>("library-phase", (e) => cb(e.payload)),
  onScanPhase: (cb: (phase: "rules" | "discovering" | "idle") => void): Promise<UnlistenFn> => listen<"rules" | "discovering" | "idle">("scan-phase", (e) => cb(e.payload)),
  onBackupProgress: (cb: (p: { index: number; total: number; item: BackupItemResult }) => void): Promise<UnlistenFn> =>
    listen("backup-progress", (e) => cb(e.payload as { index: number; total: number; item: BackupItemResult })),
};

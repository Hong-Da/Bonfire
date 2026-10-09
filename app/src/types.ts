// 与 Rust 端（bonfire-app / Tauri 命令层）JSON 结构一一对应，字段为 camelCase。

export type GameState = "never-backed-up" | "has-changes" | "up-to-date";
export type Theme = "auto" | "light" | "dark";

export interface AppInfo {
  version: string;
  portable: boolean;
  firstRun: boolean;
  locale: string;
  theme: Theme;
  configPath: string;
  defaultRepoDir: string;
  repoDir: string | null;
  configError: string | null;
  /** 本机电脑名；快照记录的电脑名与它相同即为本机备份。 */
  host: string;
  /** 存档安全保护是否开启。 */
  protectLocal: boolean;
}

export interface Config {
  general: { language?: string; theme: Theme; repo?: string; close_to_tray: boolean };
  backup: { include_config_files: boolean; size_warning_gb: number; disabled_platforms: string[]; protect_local: boolean };
  metadata: { online: boolean; igdb_client_id: string; igdb_client_secret: string; steamgriddb_api_key: string };
  log: { level: string; keep_days: number };
}

export interface ChangeSummary {
  added: number;
  modified: number;
  deleted: number;
  unchanged: number;
  changedBytes: number;
}

export interface GameRow {
  id: string;
  name: string;
  nameZh: string | null;
  platform: string;
  enabled: boolean;
  coverThumb: string | null;
  state: GameState;
  summary: ChangeSummary;
  files: number;
  totalBytes: number;
  lastModifiedNs: number | null;
  head: string | null;
  branch: string;
  detached: boolean;
  snapshots: number;
  lastBackupAt: string | null;
  cloudSave: boolean;
  playtimeMin: number | null;
  unconfirmed: number;
  /** 从别处导入、本机还没有安装的游戏为 false。 */
  installed: boolean;
  /** 本机还不能确定路径的存档位置数。 */
  unresolved: number;
}

export type FileStatus = "added" | "modified" | "deleted" | "unchanged";

export interface FileChange {
  path: string;
  status: FileStatus;
  size: number;
  hash: string | null;
}

export type Trigger = "manual" | "auto-exit" | "auto-timer" | "pre-restore" | "import";

export interface SnapshotNode {
  id: string;
  parents: string[];
  createdAt: string;
  trigger: Trigger;
  comment: string;
  files: number;
  bytes: number;
  changed: number;
  playtimeMin: number | null;
  host: string;
  refs: string[];
}

export interface RefView {
  name: string;
  kind: "branch" | "tag";
  target: string;
  current: boolean;
}

export interface GameRecord {
  id: string;
  name: string;
  platform: string;
  appId?: string;
  installDir?: string;
  enabled: boolean;
}

export interface LocationRecord {
  pathTemplate: string;
  resolvedPath: string;
  kind: "file" | "dir" | "glob" | "registry";
  tag: "save" | "config";
  source: string;
  confirmed: boolean;
}

export interface HistoryView {
  game: GameRecord;
  head: string | null;
  branch: string;
  detached: boolean;
  refs: RefView[];
  nodes: SnapshotNode[];
  locations: LocationRecord[];
}

export interface TreeEntry {
  path: string;
  blob: string;
  size: number;
  mtimeNs: number;
  tag: "save" | "config";
}

export interface DiffEntry {
  path: string;
  status: "added" | "removed" | "modified";
  sizeBefore: number | null;
  sizeAfter: number | null;
}

export type RestoreAction = "overwrite" | "create" | "delete";

export interface RestorePlan {
  target: string;
  files: { path: string; disk: string; action: RestoreAction; size: number }[];
  unchanged: number;
  partial: boolean;
  running: string[];
}

export interface RestoreReport {
  plan: RestorePlan;
  protection: string | null;
  dryRun: boolean;
}

export type BackupResult =
  | {
      result: "created";
      snapshot: string;
      branch: string;
      newBranch?: string;
      comment: string;
      summary: ChangeSummary;
      files: number;
      bytes: number;
      newBytes: number;
      warnings: string[];
    }
  | { result: "no-changes" };

export interface CmdError {
  kind: string;
  message: string;
}

export interface BackupItemResult {
  gameId: string;
  ok: boolean;
  result: BackupResult | null;
  error: CmdError | null;
}

export interface SearchHit {
  gameId: string;
  gameName: string;
  id: string;
  comment: string;
  createdAt: string;
}

export interface Profile {
  name: string;
  gameIds: string[];
  defaultComment: string;
}

export interface MetadataView {
  gameId: string;
  name: string | null;
  nameZh: string | null;
  developer: string | null;
  publisher: string | null;
  releaseDate: string | null;
  genres: string[];
  coverPath: string | null;
  heroPath: string | null;
  iconPath: string | null;
  playtimeMin: number | null;
  lastPlayedAt: number | null;
  cloudSave: boolean | null;
  lockedFields: string[];
  coverThumbPath: string | null;
}

export interface FsckReport {
  chunks: number;
  blobs: number;
  trees: number;
  snapshots: number;
  refs: number;
  errors: string[];
  unreachableSnapshots: number;
  tmpFiles: number;
}

export interface Phase {
  phase: "discovering" | "checking" | "info" | "idle";
  done: number;
  total: number;
}

export type FoundState = "new" | "managed" | "ignored";

export interface DiscoveredGame {
  id: string;
  name: string;
  nameZh: string | null;
  platform: string;
  /** 命中的规则名称；为空表示推测的位置。 */
  rule: string | null;
  state: FoundState;
  locations: string[];
  unconfirmed: number;
  files: number;
  bytes: number;
  coverThumb: string | null;
  cloudSave: boolean;
  playtimeMin: number | null;
  lastPlayed: number | null;
}

export interface NoSaveGame {
  id: string;
  name: string;
  platform: string;
  rule: string | null;
}

export interface ScanReport {
  platforms: string[];
  rules: { path: string | null; games: number; userGames: number } | null;
  games: DiscoveredGame[];
  withoutSaves: NoSaveGame[];
}

// ---------- 导出与导入 ----------

export type ExportFormat = "zip" | "folder";

export interface ExportReport {
  dest: string;
  files: number;
  bytes: number;
  snapshot: string | null;
}

export type BundleScope = { kind: "all" } | { kind: "branches"; names: string[] } | { kind: "range"; from: string | null; to: string };

export interface BundleRef {
  kind: "branch" | "tag";
  name: string;
  target: string;
}

export interface BundleReport {
  dest: string;
  snapshots: number;
  objects: number;
  bytes: number;
  refs: BundleRef[];
  prerequisites: number;
}

export type RefAction = "create" | "same" | "fast-forward" | "contained" | "diverged" | "tag-conflict";

export interface RefPlan extends BundleRef {
  action: RefAction;
  localTarget: string | null;
  newName: string | null;
  forkPoint: string | null;
}

export interface BundleNode {
  id: string;
  parents: string[];
  createdAt: string;
  trigger: Trigger;
  comment: string;
  files: number;
  bytes: number;
  host: string;
  known: boolean;
}

export interface BundlePreview {
  gameId: string;
  gameName: string;
  localGame: boolean;
  /** 导入时会把这个游戏加入游戏库（未安装）。 */
  createGame: boolean;
  createdAt: string;
  host: string;
  nodes: BundleNode[];
  newSnapshots: number;
  missing: string[];
  refs: RefPlan[];
  locationMismatch: boolean;
}

export interface BundleImportReport {
  preview: BundlePreview;
  importedSnapshots: number;
  objects: number;
  createdGame: boolean;
}

export interface CollectionEntry {
  gameId: string;
  gameName: string;
  dir: string;
  snapshots: number;
}

export interface CollectionReport {
  dest: string;
  bytes: number;
  games: CollectionEntry[];
  objects: number;
  skipped: string[];
}

/** 历史包或合集包；`key` 是包内目录（单个游戏的历史包为空）。 */
export interface ArchivePreview {
  collection: boolean;
  createdAt: string;
  host: string;
  games: { key: string; preview: BundlePreview }[];
}

export interface ArchiveImportItem {
  key: string;
  gameId: string;
  gameName: string;
  report: BundleImportReport | null;
  error: string | null;
}

export interface FilesSource {
  manifest: ExportManifest;
  gameId: string | null;
  preview: FilesPreview | null;
}

export interface FilesAutoReport {
  gameId: string;
  gameName: string;
  createdGame: boolean;
  report: ImportFilesReport;
}

export interface DeleteReport {
  removed: string[];
  rewritten: number;
  branchesDeleted: string[];
  headBranch: string | null;
  dryRun: boolean;
}

export interface ExportManifest {
  gameId: string;
  gameName: string;
  snapshot: string | null;
  snapshotCreatedAt: string | null;
  comment: string | null;
  exportedAt: string;
  host: string;
}

export interface FilesPreview {
  files: number;
  bytes: number;
  folders: { folder: string; ord: number | null; files: number }[];
  unmapped: string[];
  manifest: ExportManifest | null;
  identical: string | null;
}

export type ImportPlace = { kind: "newBranch"; name: string | null; parent: string | null } | { kind: "continue"; branch: string };

export interface ImportFilesReport {
  preview: FilesPreview;
  snapshot: string | null;
  branch: string | null;
  dryRun: boolean;
}

// 与 Rust 端（bonfire-app / Tauri 命令层）JSON 结构一一对应，字段为 camelCase。

export type GameState = "never-backed-up" | "has-changes" | "up-to-date" | "shots-only";
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
  general: { language?: string; theme: Theme; repo?: string; close_to_tray: boolean; close_asked?: boolean };
  backup: { include_config_files: boolean; size_warning_gb: number; disabled_platforms: string[]; protect_local: boolean };
  shots: ShotsConfig;
  auto_backup: AutoBackupConfig;
  metadata: { online: boolean; igdb_client_id: string; igdb_client_secret: string; steamgriddb_api_key: string };
  log: { level: string; keep_days: number };
}

export type ShotSchedule = "off" | "daily" | "weekly" | "hours";

export interface ShotsConfig {
  enabled: boolean;
  captures_dir?: string;
  schedule: ShotSchedule;
  /** `HH:MM` */
  at: string;
  /** 1 = 周一 … 7 = 周日 */
  weekday: number;
  every_hours: number;
}

/** 自动备份：定时备份与存档变化后自动备份。 */
export interface AutoBackupConfig {
  schedule: ShotSchedule;
  at: string;
  weekday: number;
  every_hours: number;
  on_change: boolean;
  check_minutes: number;
  quiet_minutes: number;
}

export interface AutoStatus {
  lastScheduled: string | null;
  lastCheck: string | null;
}

/** 一次自动备份的结果（`auto-backup` 事件）。 */
export interface AutoReport {
  kind: "scheduled" | "on-change";
  items: { gameId: string; name: string; snapshot: string | null; newBranch: string | null; error: string | null; shots: number }[];
  waiting: number;
}

/** 游戏的管理方式。 */
export type ManageMode = "both" | "saves" | "shots";

/** 图库中的一个游戏。 */
export interface GalleryGame {
  id: string;
  name: string;
  nameZh: string | null;
  platform: string;
  /** 已移出游戏库的游戏为 false（截图仍保留）。 */
  inLibrary: boolean;
  mode: ManageMode;
}

/** 图库：所有游戏的截图。 */
export interface GalleryView {
  games: GalleryGame[];
  shots: ShotRecord[];
  trash: ShotRecord[];
  bytes: number;
  trashBytes: number;
  dir: string;
}

/** 截图备份与磁盘上截图文件夹的对比。 */
export interface ShotDiff {
  gameId: string;
  new: { path: string; size: number; source: string }[];
  duplicates: number;
  ignored: { path: string; size: number; blob: string; purged: boolean }[];
  missing: ShotRecord[];
  restorable: number;
  synced: number;
}

/** 彻底删除截图的结果。 */
export interface PurgeReport {
  purged: number;
  originalsDeleted: number;
  originalsKept: number;
  originalsFailed: number;
}

export interface WriteBackReport {
  written: number;
  bytes: number;
  exists: number;
  noDir: number;
  dirs: string[];
}

/** 有截图、但还不在游戏库中的游戏。 */
export interface ShotCandidate {
  key: string;
  name: string;
  source: "steam" | "capture";
  appId: string | null;
  count: number;
  dir: string;
}

/** 相册中的一张截图。 */
export interface ShotRecord {
  gameId: string;
  blob: string;
  takenAt: string;
  source: "steam" | "capture" | "folder" | "import";
  originalPath: string;
  size: number;
  width?: number;
  height?: number;
  addedAt: string;
  /** 备份中的原图（相对游戏截图目录）。 */
  file?: string;
  /** 移入回收站的时间。 */
  deletedAt?: string;
}

export interface AlbumView {
  shots: ShotRecord[];
  /** 回收站中的截图。 */
  trash: ShotRecord[];
  trashBytes: number;
  /** 备份中的原图所在的文件夹。 */
  dir: string;
  shotsOnly: boolean;
  enabled: boolean;
  followsGlobal: boolean;
  globalEnabled: boolean;
  dirs: string[];
  pending: number;
  bytes: number;
  sources: string[];
}

export interface ShotSyncReport {
  gameId: string;
  added: number;
  bytes: number;
  disabled: boolean;
}

export interface ShotExportReport {
  dest: string;
  files: number;
  bytes: number;
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
  /** 相册中的截图数。 */
  shots: number;
  /** 只管理截图（不备份存档）。 */
  shotsOnly: boolean;
  /** 管理方式：存档和截图 / 只管理存档 / 只管理截图。 */
  mode: ManageMode;
  /** 磁盘上还没有收集的新截图数。 */
  newShots: number;
}

export type FileStatus = "added" | "modified" | "deleted" | "unchanged";

export interface FileChange {
  path: string;
  status: FileStatus;
  size: number;
  hash: string | null;
}

export type Trigger = "manual" | "auto-exit" | "auto-timer" | "auto-change" | "pre-restore" | "import";

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
  /** 顺带收集的新截图数。 */
  shots: number;
  /** 只管理截图的游戏：只收集了截图。 */
  shotsOnly: boolean;
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
  shots: number;
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
  /** 包内的截图数，其中本地相册没有的。 */
  shots: number;
  newShots: number;
}

export interface BundleImportReport {
  preview: BundlePreview;
  importedSnapshots: number;
  objects: number;
  createdGame: boolean;
  importedShots: number;
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

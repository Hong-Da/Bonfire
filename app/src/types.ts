// 与 Rust 端（bonfire-app / Tauri 命令层）JSON 结构一一对应，字段为 camelCase。

export type GameState = "never-backed-up" | "has-changes" | "up-to-date" | "shots-only";
export type Theme = "auto" | "light" | "dark";

export interface AppInfo {
  /** 仓库已加密且还没解锁（M5-2）。 */
  locked?: boolean;
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
  /** 记住游戏库的筛选 / 勾选的游戏（缺省为是）。 */
  rememberFilters?: boolean;
  rememberSelection?: boolean;
}

export interface Config {
  general: { language?: string; theme: Theme; repo?: string; close_to_tray: boolean; close_asked?: boolean; remember_filters?: boolean; remember_selection?: boolean; notify_failures?: boolean };
  backup: { include_config_files: boolean; size_warning_gb: number; disabled_platforms: string[]; protect_local: boolean; use_vss?: boolean };
  shots: ShotsConfig;
  auto_backup: AutoBackupConfig;
  retention?: RetentionConfig;
  sync?: { targets: SyncTarget[] };
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
  /** 公用截图文件夹：多个游戏的截图放在一起，按子文件夹名或文件名对应游戏 */
  shared_dirs?: string[];
  /** 备份未识别游戏的截图（放在“未识别的截图”相册） */
  keep_unsorted?: boolean;
  /** 截图变化后自动收集 */
  on_change?: boolean;
  /** 检查新截图的间隔（分钟） */
  check_minutes?: number;
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
  /** 退出游戏后自动备份 */
  on_exit?: boolean;
}

/** 历史清理（M4-4）：保留策略与自动清理。 */
export interface RetentionConfig {
  enabled: boolean;
  keep_last: number;
  keep_daily: number;
  keep_weekly: number;
  scope: "auto" | "all";
}

/** 本机操作记录（总览页）。 */
export interface Activity {
  at: string;
  action: string;
  ok: boolean;
  game?: string;
  name?: string;
  snapshot?: string;
  n?: number;
  bytes?: number;
  detail?: string;
  error?: string;
  issue?: string;
  /** 失败已处理：之后同一操作成功了，或标为已处理。 */
  resolved?: boolean;
}

export interface ActivityQuery {
  category?: string | null;
  failed?: boolean;
  game?: string | null;
  text?: string | null;
  limit?: number;
  before?: string | null;
  /** 只看最近几天（0 为全部）。 */
  days?: number;
}

export interface ActivityPage {
  items: Activity[];
  more: boolean;
  failed: number;
}

/** 需要处理的问题（红点）。 */
export interface AttentionItem {
  key: string;
  kind: "sync" | "backup" | "fsck";
  name: string;
  game: string | null;
  target: string | null;
  issue: string | null;
  error: string | null;
  at: string;
  retry: { attempt: number; at: string } | null;
}

export interface Attention {
  items: AttentionItem[];
  snoozed: number;
}

export interface GameUsage {
  id: string;
  name: string;
  snapshots: number;
  saves: number;
  shots: number;
}

export interface StorageStats {
  repoDir: string;
  total: number;
  saves: number;
  shots: number;
  other: number;
  free: number | null;
  logical: number;
  snapshots: number;
  games: GameUsage[];
  trend: [string, number][];
}

export interface GameDiskDiff {
  id: string;
  name: string;
  new: number;
  duplicates: number;
  missing: number;
  restorable: number;
  toHome: number;
  noPlace: number;
  ignored: number;
  error: string | null;
}

export interface DiskCompare {
  at: string;
  games: GameDiskDiff[];
  compared: number;
}

/** 同步失败（或提醒）的原因。 */
export type SyncIssue = "network" | "auth" | "not-found" | "public" | "size" | "file-limit" | "local" | "excluded" | "other";
/** 可以忽略的问题。 */
export const IGNORABLE: SyncIssue[] = ["network", "size", "excluded"];

/** 同步目标（REQ-015）。 */
export interface SyncTarget {
  id: string;
  name: string;
  kind: "github" | "gitee" | "webdav" | "folder";
  /** 界面上选的类型：github / gitee / webdav / cloud / network / local */
  place: string;
  url: string;
  branch: string;
  user: string;
  content: "repo" | "latest" | "shots";
  shots: boolean;
  settings: boolean;
  games: string[];
  max_file_mb: number;
  max_total_mb: number;
  schedule: "manual" | "after-backup" | "daily" | "hours";
  at: string;
  every_hours: number;
  enabled: boolean;
  ignore: SyncIssue[];
}

export interface SyncRun {
  at: string;
  uploaded: number;
  uploadedBytes: number;
  deleted: number;
  excluded: { path: string; size: number; reason: string }[];
  error?: string | null;
  issue?: SyncIssue | null;
}

export interface TargetStatus {
  target: SyncTarget;
  hasSecret: boolean;
  last: SyncRun | null;
  remoteFiles: number;
  /** 等待中的自动重试。 */
  retry: { attempt: number; at: string } | null;
}

export interface SyncPlan {
  targetId: string;
  upload: number;
  uploadBytes: number;
  delete: number;
  unchanged: number;
  excluded: { path: string; size: number; reason: string }[];
  remoteBytes: number;
  limitBytes: number;
  level: "ok" | "warn" | "over";
  parts: [string, number][];
  blocked: string | null;
  blockedIssue: SyncIssue | null;
}

export interface SyncDone {
  id: string;
  run: SyncRun | null;
  error: string | null;
}

/** 仓库加密状态（M5-2）。 */
export interface SecurityStatus {
  encrypted: boolean;
  unlocked: boolean;
  remembered: boolean;
  canRemember: boolean;
}

/** 可释放空间的一个来源。 */
export interface GcSource {
  kind: "planned" | "unreachable" | "deleted" | "shots" | "other" | "tmp";
  objects: number;
  bytes: number;
}

/** 清理历史的结果（预览时为预计）。 */
export interface CleanupReport {
  games: { gameId: string; name: string; snapshots: number; removed: string[]; rewritten: number; items?: { id: string; createdAt: string; trigger: string; comment: string }[] }[];
  removed: number;
  gc: { chunks: number; blobs: number; trees: number; snapshots: string[]; bytes: number; tmpFiles: number; sources?: GcSource[]; dryRun: boolean };
  dryRun: boolean;
}

export interface AutoStatus {
  lastScheduled: string | null;
  lastCheck: string | null;
}

/** 一次自动备份的结果（`auto-backup` 事件）。 */
export interface AutoReport {
  kind: "scheduled" | "on-change" | "on-exit";
  items: { gameId: string; name: string; snapshot: string | null; newBranch: string | null; error: string | null; shots: number }[];
  waiting: number;
  /** 正在运行、等退出后再备份的游戏 */
  deferred?: string[];
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
  /** 本机这个游戏的截图文件夹；原文件夹不在本机的截图写回到这里。 */
  home: string | null;
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
  /** 本机主存档（U-41） */
  primary: string;
  /** 当前不在本机主存档上 */
  offPrimary: boolean;
  /** 存档可能被手动替换（U-42） */
  maybeReplaced?: boolean;
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
  /** 各电脑的主存档（本机在前） */
  primaries?: PrimaryView[];
}

/** 一台电脑的主存档（U-41）。 */
export interface PrimaryView {
  key: string;
  branch: string;
  /** 电脑名 */
  label: string;
  mine: boolean;
}

export interface SetPrimaryReport {
  branch: string;
  renamed: [string, string][];
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
  /** 包内的 main 与本机分叉时导入后的名字（如 main-旧电脑） */
  mainConflict?: string | null;
}

export interface BundleImportReport {
  preview: BundlePreview;
  importedSnapshots: number;
  objects: number;
  createdGame: boolean;
  importedShots: number;
  /** 加入相册的截图（用于写到磁盘）。 */
  newShotIds: string[];
}

export interface CollectionEntry {
  gameId: string;
  gameName: string;
  dir: string;
  snapshots: number;
  shots: number;
}

/** 导出篝火包的内容。 */
export interface PackageOptions {
  saves: boolean;
  shots: boolean;
  /** 只导出这些截图（按游戏）。 */
  shotIds?: Record<string, string[]>;
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
  /** 以导入的路线为本机主存档后它的名字 */
  adoptedMain?: string | null;
}

export interface FilesSource {
  /** 存档文件夹在来源中的相对路径（根目录为空；zip 格式的篝火导出可含多个游戏） */
  dir: string;
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

/** 删除多个快照的检查与结果。 */
export interface BulkDeleteReport {
  removed: string[];
  blocked: { id: string; reason: "tag" | "protected" | "undo"; detail: string }[];
  rewritten: number;
  branches: { name: string; to: string | null; toCreatedAt: string | null }[];
  headBranch: string | null;
  currentRemoved: boolean;
  bytes: number;
  sources?: GcSource[];
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

/** 导入下载的存档（U-42）。 */
export type DownloadKind = "download" | "bonfire" | "needs-extract" | "needs-password" | "damaged";
/** 带篝火标识却按普通下载处理时的提示（U-48）：校验不通过 / 无法核对。 */
export type DownloadNotice = "modified" | "unverifiable";
export type DownloadMode = "overlay" | "replace";

export interface DownloadPreview {
  kind: DownloadKind;
  sourceName: string;
  files: { rel: string; size: number; target: string | null; ignored: boolean; replaces: boolean }[];
  locations: { ord: number; path: string }[];
  mode: DownloadMode;
  currentFiles: number;
  /** 当前存档中已有的文件（清单路径，小写） */
  existing?: string[];
  notice?: DownloadNotice | null;
}

export interface DownloadReport {
  snapshot: string | null;
  identical: string | null;
  branch: string | null;
  files: number;
}

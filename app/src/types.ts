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
}

export interface Config {
  general: { language?: string; theme: Theme; repo?: string };
  backup: { include_config_files: boolean; size_warning_gb: number; disabled_platforms: string[] };
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
}

export type FileStatus = "added" | "modified" | "deleted" | "unchanged";

export interface FileChange {
  path: string;
  status: FileStatus;
  size: number;
  hash: string | null;
}

export interface SnapshotNode {
  id: string;
  parents: string[];
  createdAt: string;
  trigger: "manual" | "auto-exit" | "auto-timer" | "pre-restore";
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

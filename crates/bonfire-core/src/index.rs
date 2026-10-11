//! SQLite 索引 `bonfire.db`（文档 §7）。
//!
//! 对象库与 refs 是唯一事实来源；数据库只存游戏配置与加速查询的索引，
//! 损坏后可由 [`reindex`] 从对象库重建（游戏配置另由应用层备份到 `games.toml`）。

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::object::{FileTag, Snapshot};
use crate::refs::RefName;
use crate::{ObjectId, Repo};

/// 数据库结构版本（PRAGMA user_version）。
/// 3：截图相册（screenshots、shot_settings）。
/// 4：截图原样存为文件（screenshots.file）、回收站（deleted_at、purged）、只管理截图的游戏（shot_settings.shots_only）。
/// 5：游戏程序（game_processes，运行检测用的进程名）。
const SCHEMA_VERSION: i32 = 5;

/// 版本 4 新增的列（旧数据库中补上）。
const COLUMNS_V4: &[(&str, &str, &str)] = &[
    ("screenshots", "file", "TEXT NOT NULL DEFAULT ''"),
    ("screenshots", "deleted_at", "TEXT"),
    ("screenshots", "purged", "INTEGER NOT NULL DEFAULT 0"),
    ("shot_settings", "shots_only", "INTEGER NOT NULL DEFAULT 0"),
];

const SHOT_COLUMNS: &str = "game_id, blob, taken_at, source, original_path, size, width, height, added_at, file, deleted_at, purged";

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS games (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    platform TEXT NOT NULL,
    app_id TEXT,
    install_dir TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    include_config INTEGER,
    custom_rule_json TEXT
);
CREATE TABLE IF NOT EXISTS save_locations (
    id INTEGER PRIMARY KEY,
    game_id TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    ord INTEGER NOT NULL,
    path_template TEXT NOT NULL,
    resolved_path TEXT NOT NULL,
    kind TEXT NOT NULL,
    tag TEXT NOT NULL,
    source TEXT NOT NULL,
    confirmed INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS save_locations_game ON save_locations(game_id, ord);
CREATE TABLE IF NOT EXISTS scan_index (
    game_id TEXT NOT NULL,
    rel_path TEXT NOT NULL,
    size INTEGER NOT NULL,
    mtime_ns INTEGER NOT NULL,
    hash TEXT NOT NULL,
    indexed_at INTEGER NOT NULL,
    PRIMARY KEY (game_id, rel_path)
);
CREATE TABLE IF NOT EXISTS snapshots (
    hash TEXT PRIMARY KEY,
    game_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    trigger TEXT NOT NULL,
    comment TEXT NOT NULL,
    files INTEGER NOT NULL,
    bytes INTEGER NOT NULL,
    changed INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS snapshots_game ON snapshots(game_id, created_at);
CREATE VIRTUAL TABLE IF NOT EXISTS snapshots_fts USING fts5(hash UNINDEXED, game_id UNINDEXED, comment, tokenize = 'trigram');
CREATE TABLE IF NOT EXISTS snapshot_parents (
    snapshot_hash TEXT NOT NULL,
    parent_hash TEXT NOT NULL,
    ord INTEGER NOT NULL,
    PRIMARY KEY (snapshot_hash, ord)
);
CREATE INDEX IF NOT EXISTS snapshot_parents_parent ON snapshot_parents(parent_hash);
CREATE TABLE IF NOT EXISTS refs (
    game_id TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    target_hash TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (game_id, kind, name)
);
CREATE TABLE IF NOT EXISTS backup_profiles (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    game_ids_json TEXT NOT NULL,
    default_comment TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS operations (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    game_id TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    status TEXT NOT NULL,
    detail_json TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS operations_game ON operations(game_id, id);
CREATE TABLE IF NOT EXISTS game_metadata (
    game_id TEXT PRIMARY KEY,
    name TEXT,
    name_zh TEXT,
    developer TEXT,
    publisher TEXT,
    release_date TEXT,
    genres_json TEXT NOT NULL DEFAULT '[]',
    cover_path TEXT,
    icon_path TEXT,
    hero_path TEXT,
    playtime_min INTEGER,
    last_played_at INTEGER,
    cloud_save INTEGER,
    external_ids_json TEXT NOT NULL DEFAULT '{}',
    field_sources_json TEXT NOT NULL DEFAULT '{}',
    locked_fields_json TEXT NOT NULL DEFAULT '[]',
    fetched_at TEXT
);
CREATE TABLE IF NOT EXISTS screenshots (
    game_id TEXT NOT NULL,
    blob TEXT NOT NULL,
    taken_at TEXT NOT NULL,
    source TEXT NOT NULL,
    original_path TEXT NOT NULL,
    size INTEGER NOT NULL,
    width INTEGER,
    height INTEGER,
    added_at TEXT NOT NULL,
    file TEXT NOT NULL DEFAULT '',
    deleted_at TEXT,
    purged INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (game_id, blob)
);
CREATE INDEX IF NOT EXISTS screenshots_taken ON screenshots(game_id, taken_at);
CREATE TABLE IF NOT EXISTS shot_settings (
    game_id TEXT PRIMARY KEY,
    enabled INTEGER,
    dirs_json TEXT NOT NULL DEFAULT '[]',
    shots_only INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS game_processes (
    game_id TEXT PRIMARY KEY,
    names_json TEXT NOT NULL DEFAULT '[]'
);
"#;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameRecord {
    pub id: String,
    pub name: String,
    pub platform: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// 安装目录，用于运行检测（可选）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_dir: Option<String>,
    pub enabled: bool,
    /// 是否备份配置文件；`None` 表示跟随全局设置。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_config: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_rule_json: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocationKind {
    File,
    Dir,
    /// 通配模式，每次扫描重新匹配。
    Glob,
    Registry,
}

impl LocationKind {
    fn as_str(self) -> &'static str {
        match self {
            LocationKind::File => "file",
            LocationKind::Dir => "dir",
            LocationKind::Glob => "glob",
            LocationKind::Registry => "registry",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "file" => LocationKind::File,
            "glob" => LocationKind::Glob,
            "registry" => LocationKind::Registry,
            _ => LocationKind::Dir,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocationRecord {
    pub path_template: String,
    pub resolved_path: String,
    pub kind: LocationKind,
    pub tag: FileTag,
    /// `user` / `manifest` / `heuristic`
    pub source: String,
    pub confirmed: bool,
}

/// 变更检测缓存的一行：上次快照（或恢复）时文件的状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub path: String,
    pub size: u64,
    pub mtime_ns: i64,
    /// 文件内容的 BLAKE3，即 Blob ID。
    pub hash: ObjectId,
    /// 写入索引的时间（纳秒），用于 racy 判断。
    pub indexed_at_ns: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRow {
    pub id: ObjectId,
    pub game_id: String,
    pub created_at: String,
    pub trigger: String,
    pub comment: String,
    pub files: u64,
    pub bytes: u64,
    pub changed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub name: String,
    pub game_ids: Vec<String>,
    pub default_comment: String,
}

/// 游戏信息（文档 4.3 game_metadata 表）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameMetadata {
    pub game_id: String,
    pub name: Option<String>,
    pub name_zh: Option<String>,
    pub developer: Option<String>,
    pub publisher: Option<String>,
    pub release_date: Option<String>,
    pub genres: Vec<String>,
    pub cover_path: Option<String>,
    pub icon_path: Option<String>,
    pub hero_path: Option<String>,
    pub playtime_min: Option<u64>,
    /// Unix 秒。
    pub last_played_at: Option<i64>,
    pub cloud_save: Option<bool>,
    /// 外部 ID：steam / igdb / sgdb …
    pub external_ids: std::collections::BTreeMap<String, String>,
    /// 每个字段的来源提供者。
    pub field_sources: std::collections::BTreeMap<String, String>,
    /// 用户手动修改、自动刷新不覆盖的字段。
    pub locked_fields: Vec<String>,
    /// 联网信息最后获取时间（RFC 3339）。
    pub fetched_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationRow {
    pub id: i64,
    pub kind: String,
    pub game_id: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: String,
    pub detail: serde_json::Value,
}

pub struct Index {
    conn: Connection,
}

/// 相册中的一张截图（文档：截图备份方案）。`blob` 是整个文件内容的哈希，同一游戏内按内容去重；
/// 原图原样保存在仓库的 `screenshots/<游戏>/<file>`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotRecord {
    pub game_id: String,
    pub blob: ObjectId,
    /// 拍摄时间（RFC 3339，含时区）。
    pub taken_at: String,
    /// `steam` / `capture` / `folder` / `import`
    pub source: String,
    /// 收集时的原始路径（只用于显示、导出时的文件名与跳过已收集的文件）。
    pub original_path: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    pub added_at: String,
    /// 备份中的原图：仓库 `screenshots/<游戏>/` 下的相对路径。
    /// 为空表示还在对象库中（早期版本），打开仓库时迁移。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file: String,
    /// 移入相册回收站的时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
    /// 已彻底删除：只保留记录，避免磁盘上的原图被再次收集。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub purged: bool,
}

/// 游戏的截图设置：是否收集（`None` 跟随全局设置）、手动添加的截图文件夹、是否只管理截图。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dirs: Vec<String>,
    /// 只管理截图：不备份、不检查存档。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shots_only: bool,
}

/// 旧数据库中补上新增的列（已存在时跳过）。
fn add_column(conn: &Connection, table: &str, col: &str, decl: &str) -> Result<()> {
    let mut st = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = st
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|c| c == col);
    if !exists {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {decl}"))?;
    }
    Ok(())
}

fn insert_shot(conn: &Connection, game_id: &str, s: &ShotRecord) -> Result<usize> {
    Ok(conn.execute(
        &format!(
            "INSERT OR IGNORE INTO screenshots ({SHOT_COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
        ),
        params![
            game_id,
            s.blob.to_string(),
            s.taken_at,
            s.source,
            s.original_path,
            s.size as i64,
            s.width.map(i64::from),
            s.height.map(i64::from),
            s.added_at,
            s.file,
            s.deleted_at,
            s.purged
        ],
    )?)
}

fn now_rfc3339() -> String {
    jiff::Timestamp::now().to_string()
}

fn parse_id(s: String) -> rusqlite::Result<ObjectId> {
    s.parse().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

impl Index {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")?;
        let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < SCHEMA_VERSION {
            conn.execute_batch(SCHEMA)?;
            for (table, col, decl) in COLUMNS_V4 {
                add_column(&conn, table, col, decl)?;
            }
            conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
        }
        Ok(Self { conn })
    }

    /// 打开仓库的索引 `bonfire.db`。
    pub fn open_repo(repo: &Repo) -> Result<Self> {
        Self::open(&repo.root().join("bonfire.db"))
    }

    // ---------- 游戏与存档位置 ----------

    pub fn upsert_game(&self, g: &GameRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO games (id, name, platform, app_id, install_dir, enabled, include_config, custom_rule_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET name = ?2, platform = ?3, app_id = ?4, install_dir = ?5,
               enabled = ?6, include_config = ?7, custom_rule_json = ?8",
            params![g.id, g.name, g.platform, g.app_id, g.install_dir, g.enabled, g.include_config, g.custom_rule_json],
        )?;
        Ok(())
    }

    fn game_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<GameRecord> {
        Ok(GameRecord {
            id: r.get(0)?,
            name: r.get(1)?,
            platform: r.get(2)?,
            app_id: r.get(3)?,
            install_dir: r.get(4)?,
            enabled: r.get(5)?,
            include_config: r.get(6)?,
            custom_rule_json: r.get(7)?,
        })
    }

    pub fn game(&self, id: &str) -> Result<Option<GameRecord>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, platform, app_id, install_dir, enabled, include_config, custom_rule_json FROM games WHERE id = ?1",
                [id],
                Self::game_from_row,
            )
            .optional()?)
    }

    pub fn games(&self) -> Result<Vec<GameRecord>> {
        let mut st = self.conn.prepare(
            "SELECT id, name, platform, app_id, install_dir, enabled, include_config, custom_rule_json FROM games ORDER BY name COLLATE NOCASE, id",
        )?;
        let rows = st
            .query_map([], Self::game_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// 删除游戏配置及其存档位置与变更检测缓存（历史快照保留在对象库中）。
    pub fn delete_game(&self, id: &str) -> Result<bool> {
        self.conn
            .execute("DELETE FROM scan_index WHERE game_id = ?1", [id])?;
        self.conn
            .execute("DELETE FROM game_processes WHERE game_id = ?1", [id])?;
        Ok(self.conn.execute("DELETE FROM games WHERE id = ?1", [id])? > 0)
    }

    pub fn set_locations(&mut self, game_id: &str, locs: &[LocationRecord]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM save_locations WHERE game_id = ?1", [game_id])?;
        for (i, l) in locs.iter().enumerate() {
            tx.execute(
                "INSERT INTO save_locations (game_id, ord, path_template, resolved_path, kind, tag, source, confirmed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![game_id, i as i64, l.path_template, l.resolved_path, l.kind.as_str(), tag_str(l.tag), l.source, l.confirmed],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 存档位置，按添加顺序（顺序号即清单中的路径前缀）。
    pub fn locations(&self, game_id: &str) -> Result<Vec<LocationRecord>> {
        let mut st = self.conn.prepare(
            "SELECT path_template, resolved_path, kind, tag, source, confirmed FROM save_locations WHERE game_id = ?1 ORDER BY ord",
        )?;
        let rows = st
            .query_map([game_id], |r| {
                Ok(LocationRecord {
                    path_template: r.get(0)?,
                    resolved_path: r.get(1)?,
                    kind: LocationKind::parse(&r.get::<_, String>(2)?),
                    tag: if r.get::<_, String>(3)? == "config" {
                        FileTag::Config
                    } else {
                        FileTag::Save
                    },
                    source: r.get(4)?,
                    confirmed: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ---------- 变更检测缓存 ----------

    pub fn scan_entries(&self, game_id: &str) -> Result<HashMap<String, IndexEntry>> {
        let mut st = self.conn.prepare(
            "SELECT rel_path, size, mtime_ns, hash, indexed_at FROM scan_index WHERE game_id = ?1",
        )?;
        let rows = st
            .query_map([game_id], |r| {
                Ok(IndexEntry {
                    path: r.get(0)?,
                    size: r.get::<_, i64>(1)? as u64,
                    mtime_ns: r.get(2)?,
                    hash: parse_id(r.get(3)?)?,
                    indexed_at_ns: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().map(|e| (e.path.clone(), e)).collect())
    }

    /// 用一组条目整体替换游戏的缓存（备份、完全恢复之后）。
    pub fn replace_scan_entries(&mut self, game_id: &str, entries: &[IndexEntry]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM scan_index WHERE game_id = ?1", [game_id])?;
        Self::insert_entries(&tx, game_id, entries)?;
        tx.commit()?;
        Ok(())
    }

    /// 更新或新增部分条目（例如内容未变、只有 mtime 变化的文件）。
    pub fn upsert_scan_entries(&mut self, game_id: &str, entries: &[IndexEntry]) -> Result<()> {
        let tx = self.conn.transaction()?;
        Self::insert_entries(&tx, game_id, entries)?;
        tx.commit()?;
        Ok(())
    }

    fn insert_entries(
        tx: &rusqlite::Transaction<'_>,
        game_id: &str,
        entries: &[IndexEntry],
    ) -> Result<()> {
        let mut st = tx.prepare(
            "INSERT OR REPLACE INTO scan_index (game_id, rel_path, size, mtime_ns, hash, indexed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for e in entries {
            st.execute(params![
                game_id,
                e.path,
                e.size as i64,
                e.mtime_ns,
                e.hash.to_string(),
                e.indexed_at_ns
            ])?;
        }
        Ok(())
    }

    // ---------- 快照 ----------

    /// 记录快照（重复记录时忽略）；`comment` 为显示用备注（有注记时取注记）。
    pub fn add_snapshot(&mut self, id: &ObjectId, s: &Snapshot, comment: &str) -> Result<()> {
        let game = s.game.clone();
        self.add_snapshot_as(id, s, comment, &game)
    }

    /// 记录快照，归到 `game_id` 下（导入的快照可能记录着另一台电脑上的游戏 ID）。
    pub fn add_snapshot_as(
        &mut self,
        id: &ObjectId,
        s: &Snapshot,
        comment: &str,
        game_id: &str,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        let trigger = serde_json::to_value(s.trigger)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO snapshots (hash, game_id, created_at, trigger, comment, files, bytes, changed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id.to_string(), game_id, s.created_at, trigger, comment, s.stats.files as i64, s.stats.bytes as i64, s.stats.changed as i64],
        )?;
        if inserted > 0 {
            tx.execute(
                "INSERT INTO snapshots_fts (hash, game_id, comment) VALUES (?1, ?2, ?3)",
                params![id.to_string(), game_id, comment],
            )?;
            for (i, p) in s.parents.iter().enumerate() {
                tx.execute(
                    "INSERT OR IGNORE INTO snapshot_parents (snapshot_hash, parent_hash, ord) VALUES (?1, ?2, ?3)",
                    params![id.to_string(), p.to_string(), i as i64],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn update_comment(&mut self, id: &ObjectId, comment: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE snapshots SET comment = ?2 WHERE hash = ?1",
            params![id.to_string(), comment],
        )?;
        tx.execute(
            "UPDATE snapshots_fts SET comment = ?2 WHERE hash = ?1",
            params![id.to_string(), comment],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn snapshot_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SnapshotRow> {
        Ok(SnapshotRow {
            id: parse_id(r.get(0)?)?,
            game_id: r.get(1)?,
            created_at: r.get(2)?,
            trigger: r.get(3)?,
            comment: r.get(4)?,
            files: r.get::<_, i64>(5)? as u64,
            bytes: r.get::<_, i64>(6)? as u64,
            changed: r.get::<_, i64>(7)? as u64,
        })
    }

    pub fn snapshot(&self, id: &ObjectId) -> Result<Option<SnapshotRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT hash, game_id, created_at, trigger, comment, files, bytes, changed FROM snapshots WHERE hash = ?1",
                [id.to_string()],
                Self::snapshot_from_row,
            )
            .optional()?)
    }

    /// 游戏的全部快照，最新的在前。
    pub fn snapshots_of(&self, game_id: &str) -> Result<Vec<SnapshotRow>> {
        let mut st = self.conn.prepare(
            "SELECT hash, game_id, created_at, trigger, comment, files, bytes, changed FROM snapshots WHERE game_id = ?1 ORDER BY created_at DESC, hash",
        )?;
        let rows = st
            .query_map([game_id], Self::snapshot_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// 备注全文搜索（子串匹配，不区分大小写）。少于 3 个字符时退回 LIKE。
    pub fn search_comments(&self, query: &str, game_id: Option<&str>) -> Result<Vec<SnapshotRow>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let cols =
            "s.hash, s.game_id, s.created_at, s.trigger, s.comment, s.files, s.bytes, s.changed";
        let rows = if q.chars().count() >= 3 {
            let phrase = format!("\"{}\"", q.replace('"', "\"\""));
            let mut st = self.conn.prepare(&format!(
                "SELECT {cols} FROM snapshots_fts f JOIN snapshots s ON s.hash = f.hash
                 WHERE snapshots_fts MATCH ?1 AND (?2 IS NULL OR s.game_id = ?2) ORDER BY s.created_at DESC"
            ))?;
            st.query_map(
                params![format!("comment : {phrase}"), game_id],
                Self::snapshot_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let pattern = format!(
                "%{}%",
                q.replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            );
            let mut st = self.conn.prepare(&format!(
                "SELECT {cols} FROM snapshots s WHERE s.comment LIKE ?1 ESCAPE '\\' AND (?2 IS NULL OR s.game_id = ?2) ORDER BY s.created_at DESC"
            ))?;
            st.query_map(params![pattern, game_id], Self::snapshot_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(rows)
    }

    /// 删除快照记录（快照对象已被删除时）。
    pub fn delete_snapshots(&mut self, ids: &[ObjectId]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for id in ids {
            let h = id.to_string();
            tx.execute("DELETE FROM snapshots WHERE hash = ?1", [&h])?;
            tx.execute("DELETE FROM snapshots_fts WHERE hash = ?1", [&h])?;
            tx.execute(
                "DELETE FROM snapshot_parents WHERE snapshot_hash = ?1",
                [&h],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ---------- 截图相册 ----------

    fn shot_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ShotRecord> {
        Ok(ShotRecord {
            game_id: r.get(0)?,
            blob: parse_id(r.get(1)?)?,
            taken_at: r.get(2)?,
            source: r.get(3)?,
            original_path: r.get(4)?,
            size: r.get::<_, i64>(5)? as u64,
            width: r.get::<_, Option<i64>>(6)?.map(|v| v as u32),
            height: r.get::<_, Option<i64>>(7)?.map(|v| v as u32),
            added_at: r.get(8)?,
            file: r.get(9)?,
            deleted_at: r.get(10)?,
            purged: r.get(11)?,
        })
    }

    /// 记录截图；同一游戏中内容相同的已存在时不记录，返回 false。
    pub fn add_shot(&self, s: &ShotRecord) -> Result<bool> {
        Ok(insert_shot(&self.conn, &s.game_id, s)? > 0)
    }

    /// 更新截图的存放位置与回收站状态。
    pub fn update_shot(&self, s: &ShotRecord) -> Result<()> {
        self.conn.execute(
            "UPDATE screenshots SET file = ?3, deleted_at = ?4, purged = ?5, original_path = ?6 WHERE game_id = ?1 AND blob = ?2",
            params![s.game_id, s.blob.to_string(), s.file, s.deleted_at, s.purged, s.original_path],
        )?;
        Ok(())
    }

    /// 还没有迁移为文件的截图（早期版本存放在对象库中）。
    pub fn shots_without_file(&self) -> Result<Vec<ShotRecord>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {SHOT_COLUMNS} FROM screenshots WHERE file = '' AND purged = 0"
        ))?;
        let rows = st
            .query_map([], Self::shot_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// 游戏的全部截图，最新拍摄的在前。
    /// 相册中所有截图的文件 ID（垃圾回收归类早期版本留下的截图对象用）。
    pub fn all_shot_blobs(&self) -> Result<Vec<ObjectId>> {
        let mut stmt = self.conn.prepare("SELECT DISTINCT blob FROM screenshots")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.flatten().filter_map(|s| s.parse().ok()).collect())
    }

    pub fn shots_of(&self, game_id: &str) -> Result<Vec<ShotRecord>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {SHOT_COLUMNS} FROM screenshots WHERE game_id = ?1 ORDER BY taken_at DESC, blob"
        ))?;
        let rows = st
            .query_map([game_id], Self::shot_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn delete_shots(&self, game_id: &str, blobs: &[ObjectId]) -> Result<usize> {
        let mut n = 0;
        for b in blobs {
            n += self.conn.execute(
                "DELETE FROM screenshots WHERE game_id = ?1 AND blob = ?2",
                params![game_id, b.to_string()],
            )?;
        }
        Ok(n)
    }

    /// 用相册文件的内容整体替换游戏的截图记录（重建索引时）。
    pub fn replace_shots(&mut self, game_id: &str, shots: &[ShotRecord]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM screenshots WHERE game_id = ?1", [game_id])?;
        for s in shots {
            insert_shot(&tx, game_id, s)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn shot_settings(&self, game_id: &str) -> Result<ShotSettings> {
        let row: Option<(Option<bool>, String, bool)> = self
            .conn
            .query_row(
                "SELECT enabled, dirs_json, shots_only FROM shot_settings WHERE game_id = ?1",
                [game_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(match row {
            Some((enabled, dirs, shots_only)) => ShotSettings {
                enabled,
                dirs: serde_json::from_str(&dirs).unwrap_or_default(),
                shots_only,
            },
            None => ShotSettings::default(),
        })
    }

    /// 游戏程序：运行检测用的进程名（如 `Game.exe`），用户添加。
    pub fn processes(&self, game_id: &str) -> Result<Vec<String>> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT names_json FROM game_processes WHERE game_id = ?1",
                [game_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(json
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default())
    }

    pub fn set_processes(&self, game_id: &str, names: &[String]) -> Result<()> {
        if names.is_empty() {
            self.conn
                .execute("DELETE FROM game_processes WHERE game_id = ?1", [game_id])?;
        } else {
            self.conn.execute(
                "INSERT INTO game_processes (game_id, names_json) VALUES (?1, ?2)
                 ON CONFLICT(game_id) DO UPDATE SET names_json = ?2",
                params![game_id, serde_json::to_string(names).expect("可以序列化")],
            )?;
        }
        Ok(())
    }

    pub fn save_shot_settings(&self, game_id: &str, s: &ShotSettings) -> Result<()> {
        self.conn.execute(
            "INSERT INTO shot_settings (game_id, enabled, dirs_json, shots_only) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(game_id) DO UPDATE SET enabled = ?2, dirs_json = ?3, shots_only = ?4",
            params![
                game_id,
                s.enabled,
                serde_json::to_string(&s.dirs).expect("可以序列化"),
                s.shots_only
            ],
        )?;
        Ok(())
    }

    /// 以某快照为父的快照（用于历史图）。
    pub fn children(&self, id: &ObjectId) -> Result<Vec<ObjectId>> {
        let mut st = self.conn.prepare("SELECT snapshot_hash FROM snapshot_parents WHERE parent_hash = ?1 ORDER BY snapshot_hash")?;
        let rows = st
            .query_map([id.to_string()], |r| parse_id(r.get(0)?))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// 用当前引用整体替换游戏的 refs 镜像。
    pub fn set_refs(&mut self, game_id: &str, refs: &[(RefName, ObjectId)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM refs WHERE game_id = ?1", [game_id])?;
        let now = now_rfc3339();
        for (r, id) in refs {
            let kind = serde_json::to_value(r.kind)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default();
            tx.execute(
                "INSERT INTO refs (game_id, name, kind, target_hash, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![game_id, r.name, kind, id.to_string(), now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ---------- 备份方案 ----------

    pub fn save_profile(&self, p: &Profile) -> Result<()> {
        self.conn.execute(
            "INSERT INTO backup_profiles (name, game_ids_json, default_comment) VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET game_ids_json = ?2, default_comment = ?3",
            params![
                p.name,
                serde_json::to_string(&p.game_ids).unwrap(),
                p.default_comment
            ],
        )?;
        Ok(())
    }

    pub fn profiles(&self) -> Result<Vec<Profile>> {
        let mut st = self.conn.prepare(
            "SELECT name, game_ids_json, default_comment FROM backup_profiles ORDER BY name",
        )?;
        let rows = st
            .query_map([], |r| {
                let ids: String = r.get(1)?;
                Ok(Profile {
                    name: r.get(0)?,
                    game_ids: serde_json::from_str(&ids).unwrap_or_default(),
                    default_comment: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn profile(&self, name: &str) -> Result<Option<Profile>> {
        Ok(self.profiles()?.into_iter().find(|p| p.name == name))
    }

    pub fn delete_profile(&self, name: &str) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM backup_profiles WHERE name = ?1", [name])?
            > 0)
    }

    // ---------- 游戏信息 ----------

    pub fn metadata(&self, game_id: &str) -> Result<Option<GameMetadata>> {
        Ok(self
            .conn
            .query_row(
                "SELECT game_id, name, name_zh, developer, publisher, release_date, genres_json, cover_path, icon_path, hero_path,
                        playtime_min, last_played_at, cloud_save, external_ids_json, field_sources_json, locked_fields_json, fetched_at
                 FROM game_metadata WHERE game_id = ?1",
                [game_id],
                |r| {
                    Ok(GameMetadata {
                        game_id: r.get(0)?,
                        name: r.get(1)?,
                        name_zh: r.get(2)?,
                        developer: r.get(3)?,
                        publisher: r.get(4)?,
                        release_date: r.get(5)?,
                        genres: from_json(&r.get::<_, String>(6)?),
                        cover_path: r.get(7)?,
                        icon_path: r.get(8)?,
                        hero_path: r.get(9)?,
                        playtime_min: r.get::<_, Option<i64>>(10)?.map(|v| v as u64),
                        last_played_at: r.get(11)?,
                        cloud_save: r.get(12)?,
                        external_ids: from_json(&r.get::<_, String>(13)?),
                        field_sources: from_json(&r.get::<_, String>(14)?),
                        locked_fields: from_json(&r.get::<_, String>(15)?),
                        fetched_at: r.get(16)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn save_metadata(&self, m: &GameMetadata) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO game_metadata (game_id, name, name_zh, developer, publisher, release_date, genres_json, cover_path, icon_path, hero_path,
                playtime_min, last_played_at, cloud_save, external_ids_json, field_sources_json, locked_fields_json, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                m.game_id,
                m.name,
                m.name_zh,
                m.developer,
                m.publisher,
                m.release_date,
                to_json(&m.genres),
                m.cover_path,
                m.icon_path,
                m.hero_path,
                m.playtime_min.map(|v| v as i64),
                m.last_played_at,
                m.cloud_save,
                to_json(&m.external_ids),
                to_json(&m.field_sources),
                to_json(&m.locked_fields),
                m.fetched_at,
            ],
        )?;
        Ok(())
    }

    // ---------- 操作日志 ----------

    pub fn begin_op(
        &self,
        kind: &str,
        game_id: Option<&str>,
        detail: &serde_json::Value,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO operations (kind, game_id, started_at, status, detail_json) VALUES (?1, ?2, ?3, 'running', ?4)",
            params![kind, game_id, now_rfc3339(), detail.to_string()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn finish_op(&self, id: i64, status: &str, detail: &serde_json::Value) -> Result<()> {
        self.conn.execute(
            "UPDATE operations SET finished_at = ?2, status = ?3, detail_json = ?4 WHERE id = ?1",
            params![id, now_rfc3339(), status, detail.to_string()],
        )?;
        Ok(())
    }

    /// 只改操作详情（快照 ID 因清理历史改变时）。
    pub fn set_op_detail(&self, id: i64, detail: &serde_json::Value) -> Result<()> {
        self.conn.execute(
            "UPDATE operations SET detail_json = ?2 WHERE id = ?1",
            params![id, detail.to_string()],
        )?;
        Ok(())
    }

    pub fn set_op_status(&self, id: i64, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE operations SET status = ?2 WHERE id = ?1",
            params![id, status],
        )?;
        Ok(())
    }

    /// 游戏最近的一条操作（任意类型）。
    pub fn last_op(&self, game_id: &str) -> Result<Option<OperationRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, kind, game_id, started_at, finished_at, status, detail_json FROM operations WHERE game_id = ?1 ORDER BY id DESC LIMIT 1",
                [game_id],
                |r| {
                    let detail: String = r.get(6)?;
                    Ok(OperationRow {
                        id: r.get(0)?,
                        kind: r.get(1)?,
                        game_id: r.get(2)?,
                        started_at: r.get(3)?,
                        finished_at: r.get(4)?,
                        status: r.get(5)?,
                        detail: serde_json::from_str(&detail).unwrap_or_default(),
                    })
                },
            )
            .optional()?)
    }
}

fn to_json<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).expect("序列化不会失败")
}

/// 解析 JSON 列；格式错误时取默认值（索引可重建，不让单行坏数据阻塞读取）。
fn from_json<T: serde::de::DeserializeOwned + Default>(s: &str) -> T {
    serde_json::from_str(s).unwrap_or_default()
}

pub(crate) fn tag_str(t: FileTag) -> &'static str {
    match t {
        FileTag::Save => "save",
        FileTag::Config => "config",
    }
}

/// 重建统计。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReindexStats {
    pub snapshots: u64,
    pub refs: u64,
    pub games_with_history: u64,
}

/// 从对象库与 refs 重建快照、引用与变更检测缓存（文档 §7）。游戏配置由调用方另行恢复。
pub fn reindex(repo: &Repo, index: &mut Index) -> Result<ReindexStats> {
    let mut stats = ReindexStats::default();
    index.conn.execute_batch(
        "DELETE FROM snapshots; DELETE FROM snapshots_fts; DELETE FROM snapshot_parents; DELETE FROM refs; DELETE FROM scan_index;",
    )?;
    let mut alias_of = std::collections::HashMap::new();
    for game in repo.games()? {
        for a in repo.aliases(&game)? {
            alias_of.insert(a, game.clone());
        }
    }
    for id in repo.list_objects(crate::object::Kind::Snapshot)? {
        let snap = repo.get_snapshot(&id)?;
        let comment = repo.effective_comment(&id, &snap)?;
        let game = alias_of
            .get(&snap.game)
            .cloned()
            .unwrap_or_else(|| snap.game.clone());
        index.add_snapshot_as(&id, &snap, &comment, &game)?;
        stats.snapshots += 1;
    }
    for game in repo.games()? {
        let refs = repo.list_refs(&game)?;
        stats.refs += refs.len() as u64;
        index.set_refs(&game, &refs)?;
        // 变更检测缓存取当前状态对应快照的清单：Blob ID 就是文件内容哈希。
        if let Some(head) = repo.head_snapshot(&game)? {
            let tree = repo.get_tree(&repo.get_snapshot(&head)?.tree)?;
            let entries: Vec<IndexEntry> = tree
                .entries
                .iter()
                .map(|e| IndexEntry {
                    path: e.path.clone(),
                    size: e.size,
                    mtime_ns: e.mtime_ns,
                    hash: e.blob,
                    indexed_at_ns: 0,
                })
                .collect();
            index.replace_scan_entries(&game, &entries)?;
            stats.games_with_history += 1;
        }
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{SnapshotStats, Tree, TreeEntry, Trigger};

    fn setup() -> (tempfile::TempDir, Repo, Index) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repo::init(dir.path()).unwrap();
        let index = Index::open_repo(&repo).unwrap();
        (dir, repo, index)
    }

    fn snapshot(game: &str, comment: &str, created_at: &str) -> Snapshot {
        Snapshot {
            game: game.into(),
            tree: ObjectId::of(b"tree"),
            parents: vec![],
            created_at: created_at.into(),
            trigger: Trigger::Manual,
            host: "PC".into(),
            comment: comment.into(),
            stats: SnapshotStats {
                files: 2,
                bytes: 10,
                changed: 1,
            },
            playtime_min: None,
        }
    }

    #[test]
    fn fts5_trigram_is_available_and_matches_chinese_substrings() {
        let (_d, _r, mut idx) = setup();
        let a = ObjectId::of(b"a");
        let b = ObjectId::of(b"b");
        idx.add_snapshot(
            &a,
            &snapshot(
                "g1",
                "打完大树守卫，强化 +5 前",
                "2026-10-01T10:00:00+08:00",
            ),
            "打完大树守卫，强化 +5 前",
        )
        .unwrap();
        idx.add_snapshot(
            &b,
            &snapshot("g2", "Before the BOSS fight", "2026-10-02T10:00:00+08:00"),
            "Before the BOSS fight",
        )
        .unwrap();
        fn ids(idx: &Index, q: &str, g: Option<&str>) -> Vec<ObjectId> {
            idx.search_comments(q, g)
                .unwrap()
                .into_iter()
                .map(|r| r.id)
                .collect()
        }
        assert_eq!(ids(&idx, "大树守卫", None), [a]);
        assert_eq!(ids(&idx, "大树", None), [a], "两个字退回 LIKE");
        assert_eq!(ids(&idx, "boss", None), [b], "不区分大小写");
        assert_eq!(ids(&idx, "boss", Some("g1")), Vec::<ObjectId>::new());
        assert_eq!(
            ids(&idx, "\"引号", None),
            Vec::<ObjectId>::new(),
            "特殊字符不报错"
        );
        assert_eq!(ids(&idx, "100%", None), Vec::<ObjectId>::new());

        idx.update_comment(&a, "新的备注 ã ç é").unwrap();
        assert_eq!(ids(&idx, "大树守卫", None), Vec::<ObjectId>::new());
        assert_eq!(ids(&idx, "ã ç", None), [a]);
    }

    #[test]
    fn games_locations_and_cascade() {
        let (_d, _r, mut idx) = setup();
        let g = GameRecord {
            id: "custom-1".into(),
            name: "空洞骑士".into(),
            platform: "custom".into(),
            app_id: None,
            install_dir: Some("D:/Games/HK".into()),
            enabled: true,
            include_config: None,
            custom_rule_json: None,
        };
        idx.upsert_game(&g).unwrap();
        let locs = vec![
            LocationRecord {
                path_template: "D:/saves".into(),
                resolved_path: "D:/saves".into(),
                kind: LocationKind::Dir,
                tag: FileTag::Save,
                source: "user".into(),
                confirmed: true,
            },
            LocationRecord {
                path_template: "D:/cfg.ini".into(),
                resolved_path: "D:/cfg.ini".into(),
                kind: LocationKind::File,
                tag: FileTag::Config,
                source: "user".into(),
                confirmed: true,
            },
        ];
        idx.set_locations("custom-1", &locs).unwrap();
        assert_eq!(idx.game("custom-1").unwrap(), Some(g.clone()));
        assert_eq!(idx.locations("custom-1").unwrap(), locs);
        assert!(idx.delete_game("custom-1").unwrap());
        assert!(
            idx.locations("custom-1").unwrap().is_empty(),
            "存档位置随游戏删除"
        );
        assert_eq!(idx.game("custom-1").unwrap(), None);
    }

    #[test]
    fn scan_entries_replace_and_upsert() {
        let (_d, _r, mut idx) = setup();
        let e = |p: &str, size| IndexEntry {
            path: p.into(),
            size,
            mtime_ns: 1,
            hash: ObjectId::of(p.as_bytes()),
            indexed_at_ns: 2,
        };
        idx.replace_scan_entries("g", &[e("0/a", 1), e("0/b", 2)])
            .unwrap();
        idx.upsert_scan_entries("g", &[e("0/b", 3)]).unwrap();
        let m = idx.scan_entries("g").unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m["0/b"].size, 3);
        idx.replace_scan_entries("g", &[e("0/c", 1)]).unwrap();
        assert_eq!(
            idx.scan_entries("g").unwrap().keys().collect::<Vec<_>>(),
            ["0/c"]
        );
    }

    #[test]
    fn metadata_roundtrip_and_v1_upgrade() {
        let (dir, _r, idx) = setup();
        let mut m = GameMetadata {
            game_id: "steam-1".into(),
            name: Some("Hollow Knight".into()),
            name_zh: Some("空洞骑士".into()),
            ..Default::default()
        };
        m.genres = vec!["动作".into()];
        m.external_ids.insert("steam".into(), "1".into());
        m.locked_fields.push("name".into());
        m.playtime_min = Some(2520);
        m.cloud_save = Some(true);
        idx.save_metadata(&m).unwrap();
        assert_eq!(idx.metadata("steam-1").unwrap(), Some(m));
        assert_eq!(idx.metadata("nope").unwrap(), None);
        // 模拟旧版本数据库：删除新表并把版本改回 1，重新打开后自动升级。
        idx.conn
            .execute_batch("DROP TABLE game_metadata; PRAGMA user_version = 1;")
            .unwrap();
        drop(idx);
        let idx = Index::open(&dir.path().join("bonfire.db")).unwrap();
        assert_eq!(idx.metadata("steam-1").unwrap(), None);
    }

    #[test]
    fn profiles_and_operations() {
        let (_d, _r, idx) = setup();
        idx.save_profile(&Profile {
            name: "周末".into(),
            game_ids: vec!["a".into(), "b".into()],
            default_comment: "".into(),
        })
        .unwrap();
        idx.save_profile(&Profile {
            name: "周末".into(),
            game_ids: vec!["a".into()],
            default_comment: "x".into(),
        })
        .unwrap();
        assert_eq!(idx.profiles().unwrap().len(), 1);
        assert_eq!(idx.profile("周末").unwrap().unwrap().game_ids, ["a"]);
        assert!(idx.delete_profile("周末").unwrap());

        let id = idx
            .begin_op("restore", Some("g"), &serde_json::json!({"target": "x"}))
            .unwrap();
        idx.finish_op(id, "done", &serde_json::json!({"target": "x", "files": 3}))
            .unwrap();
        let op = idx.last_op("g").unwrap().unwrap();
        assert_eq!((op.kind.as_str(), op.status.as_str()), ("restore", "done"));
        assert_eq!(op.detail["files"], 3);
    }

    #[test]
    fn reindex_rebuilds_from_objects() {
        let (_d, repo, mut idx) = setup();
        let lock = repo.lock().unwrap();
        let blob = repo.write_file(&b"save"[..], Path::new("s")).unwrap().blob;
        let tree = repo
            .put_tree(&Tree::new(vec![TreeEntry {
                path: "0/s.sav".into(),
                blob,
                size: 4,
                mtime_ns: 99,
                tag: FileTag::Save,
            }]))
            .unwrap();
        let mut s = snapshot("custom-g", "第一次", "2026-10-09T10:00:00+08:00");
        s.tree = tree;
        let id = repo.put_snapshot(&s).unwrap();
        repo.update_ref(
            &lock,
            "custom-g",
            &RefName::branch("main"),
            None,
            id,
            "backup",
            "",
        )
        .unwrap();
        repo.set_note(&lock, &id, "改过的备注").unwrap();

        let stats = reindex(&repo, &mut idx).unwrap();
        assert_eq!(
            stats,
            ReindexStats {
                snapshots: 1,
                refs: 1,
                games_with_history: 1
            }
        );
        assert_eq!(idx.snapshot(&id).unwrap().unwrap().comment, "改过的备注");
        let entries = idx.scan_entries("custom-g").unwrap();
        assert_eq!(entries["0/s.sav"].hash, blob);
        assert_eq!(entries["0/s.sav"].mtime_ns, 99);
        // 删除数据库后重建得到同样的结果。
        drop(idx);
        std::fs::remove_file(repo.root().join("bonfire.db")).unwrap();
        let _ = std::fs::remove_file(repo.root().join("bonfire.db-wal"));
        let _ = std::fs::remove_file(repo.root().join("bonfire.db-shm"));
        let mut idx = Index::open_repo(&repo).unwrap();
        reindex(&repo, &mut idx).unwrap();
        assert_eq!(idx.snapshots_of("custom-g").unwrap().len(), 1);
    }
}

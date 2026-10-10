//! 扫描状态与备份服务（文档 §5、8.1）。
//!
//! 写入顺序固定为 Chunk → Blob → Tree → Snapshot → 更新 ref → 更新索引；
//! 任何一步失败都不会破坏已有历史。

use std::collections::HashMap;

use bonfire_core::index::{GameRecord, IndexEntry};
use bonfire_core::refs::{Head, RefName, validate_ref_name};
use bonfire_core::scan::{self, ChangeSummary, DiskFile, FileChange, FileStatus};
use bonfire_core::{Kind, ObjectId, RepoLock, Snapshot, SnapshotStats, Tree, TreeEntry, Trigger};
use serde::Serialize;

use crate::app::App;
use crate::error::{AppError, Result};
use crate::i18n::{t, tf};
use crate::util::{host_name, human_bytes, now_compact, now_local_rfc3339};

/// 备注长度上限（文档 8.3）。
pub const MAX_COMMENT_CHARS: usize = 500;
const STABLE_READ_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GameState {
    /// 从未快照。
    NeverBackedUp,
    /// 有新增、修改或删除的文件。
    HasChanges,
    UpToDate,
    /// 只管理截图：不备份、不检查存档。
    ShotsOnly,
}

#[derive(Debug, Clone, Serialize)]
pub struct GameStatus {
    pub game: GameRecord,
    pub state: GameState,
    pub summary: ChangeSummary,
    pub changes: Vec<FileChange>,
    /// 当前状态对应的快照。
    pub head: Option<ObjectId>,
    pub branch: String,
    /// 已从旧快照恢复、尚未再次备份。
    pub detached: bool,
    pub files: u64,
    pub total_bytes: u64,
    /// 存档文件中最新的修改时间（Unix 纳秒）。
    pub last_modified_ns: Option<i64>,
}

/// 从旧快照恢复后再次备份时的分支处理（文档 6.4）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DetachedChoice {
    /// 创建新分支（默认），名称为空时自动生成。
    #[default]
    NewBranch,
    NamedBranch(String),
    /// 移动当前分支，原分支后续快照只由 reflog 保留。
    MoveCurrent,
}

#[derive(Debug, Clone)]
pub struct BackupOptions {
    pub comment: Option<String>,
    pub tag: Option<String>,
    /// 无变化时也生成快照（用于只打备注或标签）。
    pub force: bool,
    pub trigger: Trigger,
    pub on_detached: DetachedChoice,
}

impl Default for BackupOptions {
    fn default() -> Self {
        Self {
            comment: None,
            tag: None,
            force: false,
            trigger: Trigger::Manual,
            on_detached: DetachedChoice::NewBranch,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "result",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum BackupResult {
    Created {
        snapshot: ObjectId,
        branch: String,
        /// 本次新建的分支（从旧快照恢复后备份时）。
        #[serde(skip_serializing_if = "Option::is_none")]
        new_branch: Option<String>,
        comment: String,
        summary: ChangeSummary,
        files: u64,
        bytes: u64,
        /// 本次新写入仓库的字节数（未压缩）。
        new_bytes: u64,
        warnings: Vec<String>,
    },
    /// 与上一快照相同，没有生成新快照。
    NoChanges,
}

impl App {
    /// 当前磁盘与上次快照的对比。
    pub fn status(&self, game: &GameRecord) -> Result<GameStatus> {
        self.status_with(game, false)
    }

    fn status_with(&self, game: &GameRecord, force_verify: bool) -> Result<GameStatus> {
        let head = self.repo.head(&game.id)?;
        let head_snap = self.repo.head_snapshot(&game.id)?;
        if self.is_shots_only(&game.id)? {
            return Ok(GameStatus {
                game: game.clone(),
                state: GameState::ShotsOnly,
                summary: ChangeSummary::default(),
                changes: Vec::new(),
                head: head_snap,
                branch: head.branch,
                detached: head.detached.is_some(),
                files: 0,
                total_bytes: 0,
                last_modified_ns: None,
            });
        }
        let (disk, changes) = self.detect(game, force_verify)?;
        let summary = ChangeSummary::of(&changes);
        let state = match head_snap {
            None => GameState::NeverBackedUp,
            Some(_) if summary.has_changes() => GameState::HasChanges,
            Some(_) => GameState::UpToDate,
        };
        Ok(GameStatus {
            game: game.clone(),
            state,
            summary,
            changes,
            head: head_snap,
            branch: head.branch,
            detached: head.detached.is_some(),
            files: disk.len() as u64,
            total_bytes: disk.iter().map(|f| f.size).sum(),
            last_modified_ns: disk.iter().map(|f| f.mtime_ns).max(),
        })
    }

    /// 与 [`App::status`] 相同，但忽略文件元数据、重新计算所有哈希（强制校验）。
    pub fn status_verified(&self, game: &GameRecord) -> Result<GameStatus> {
        self.status_with(game, true)
    }

    pub(crate) fn detect(
        &self,
        game: &GameRecord,
        force_verify: bool,
    ) -> Result<(Vec<DiskFile>, Vec<FileChange>)> {
        let disk = scan::collect_files(&self.scan_roots(game)?)?;
        let mut index = self.index.scan_entries(&game.id)?;
        // 本机还不能确定路径的位置（导入的未安装游戏）：看不到不等于删除，不参与比较。
        let held = self.held_prefixes(&game.id)?;
        index.retain(|path, _| !held.iter().any(|p| path.starts_with(p.as_str())));
        let changes = scan::detect(&disk, &index, force_verify)?;
        Ok((disk, changes))
    }

    /// 本机还不能确定路径的存档位置的清单路径前缀（如 `0/`）。
    pub(crate) fn held_prefixes(&self, game_id: &str) -> Result<Vec<String>> {
        Ok(self
            .locations(game_id)?
            .iter()
            .enumerate()
            .filter(|(_, l)| l.resolved_path.is_empty())
            .map(|(ord, _)| format!("{ord}/"))
            .collect())
    }

    /// 备份一个游戏。调用方持有仓库锁。
    pub fn backup(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        opts: &BackupOptions,
    ) -> Result<BackupResult> {
        if let Some(c) = &opts.comment {
            check_comment(c)?;
        }
        if let Some(tag) = &opts.tag {
            validate_ref_name(tag)?;
        }
        if self.is_shots_only(&game.id)? {
            return Err(AppError::Invalid(format!(
                "「{}」只管理截图，不备份存档",
                game.name
            )));
        }
        let (disk, changes) = self.detect(game, false)?;
        let summary = ChangeSummary::of(&changes);
        let head = self.repo.head(&game.id)?;
        let parent = self.repo.head_snapshot(&game.id)?;
        if parent.is_some() && !summary.has_changes() && !opts.force {
            self.refresh_index(&game.id, &disk, &changes)?;
            return Ok(BackupResult::NoChanges);
        }

        let op = self
            .index
            .begin_op("backup", Some(&game.id), &serde_json::json!({}))?;
        let result = self.backup_inner(lock, game, opts, &disk, &changes, summary, head, parent);
        let detail = match &result {
            Ok(BackupResult::Created { snapshot, .. }) => {
                serde_json::json!({ "snapshot": snapshot })
            }
            Ok(BackupResult::NoChanges) => serde_json::json!({ "noChanges": true }),
            Err(e) => serde_json::json!({ "error": e.to_string() }),
        };
        self.index
            .finish_op(op, if result.is_ok() { "done" } else { "failed" }, &detail)?;
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn backup_inner(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        opts: &BackupOptions,
        disk: &[DiskFile],
        changes: &[FileChange],
        summary: ChangeSummary,
        head: Head,
        parent: Option<ObjectId>,
    ) -> Result<BackupResult> {
        let known: HashMap<&str, &FileChange> =
            changes.iter().map(|c| (c.path.as_str(), c)).collect();
        let mut entries = Vec::with_capacity(disk.len());
        let mut new_bytes = 0u64;
        for f in disk {
            let reuse = known
                .get(f.path.as_str())
                .filter(|c| c.status == FileStatus::Unchanged)
                .and_then(|c| c.hash)
                .filter(|h| self.repo.has(Kind::Blob, h));
            let (blob, size, mtime_ns) = match reuse {
                Some(h) => (h, f.size, f.mtime_ns),
                None => {
                    let (blob, size, mtime, added) = self.store_file(f)?;
                    new_bytes += added;
                    (blob, size, mtime)
                }
            };
            entries.push(TreeEntry {
                path: f.path.clone(),
                blob,
                size,
                mtime_ns,
                tag: f.tag,
            });
        }
        // 本机还不能确定路径的位置：沿用上一个快照中的文件，不当作删除。
        let held = self.held_prefixes(&game.id)?;
        if let Some(p) = parent
            && !held.is_empty()
        {
            let prev = self.repo.get_tree(&self.repo.get_snapshot(&p)?.tree)?;
            entries.extend(
                prev.entries
                    .into_iter()
                    .filter(|e| held.iter().any(|h| e.path.starts_with(h.as_str()))),
            );
        }
        let tree = Tree::new(entries);
        let tree_id = self.repo.put_tree(&tree)?;
        if let Some(p) = parent
            && !opts.force
            && self.repo.get_snapshot(&p)?.tree == tree_id
        {
            self.write_index_from_tree(&game.id, &tree)?;
            return Ok(BackupResult::NoChanges);
        }

        let mut warnings = Vec::new();
        let total = tree.total_size();
        let limit = (self.config.backup.size_warning_gb * 1024.0 * 1024.0 * 1024.0) as u64;
        if total > limit {
            warnings.push(format!(
                "存档共 {}，超过 {} GB 的提示阈值，请确认存档位置没有误包含整个安装目录",
                human_bytes(total),
                self.config.backup.size_warning_gb
            ));
        }

        let comment = match opts.comment.as_deref().map(str::trim) {
            Some(c) if !c.is_empty() => c.to_string(),
            _ if !summary.has_changes() => t(self.locale, "backup.auto.forced").to_string(),
            _ => self.auto_comment(&summary, parent.is_none(), tree.entries.len() as u64, total),
        };
        let snap = Snapshot {
            game: game.id.clone(),
            tree: tree_id,
            parents: parent.into_iter().collect(),
            created_at: now_local_rfc3339(),
            trigger: opts.trigger,
            host: host_name(),
            comment: comment.clone(),
            stats: SnapshotStats {
                files: tree.entries.len() as u64,
                bytes: total,
                changed: summary.changed_files(),
            },
            // 文档 4.3：快照记录当时的游戏时长，便于显示“已玩 42 小时时的存档”。
            playtime_min: self.playtime_of(&game.id),
        };
        let snap_id = self.repo.put_snapshot(&snap)?;

        // 移动分支（最后一步才改指针）。
        let mut branch = head.branch.clone();
        let mut new_branch = None;
        let action = match opts.trigger {
            Trigger::PreRestore => "pre-restore",
            Trigger::Manual => "backup",
            _ => "auto-backup",
        };
        if head.detached.is_some() {
            match &opts.on_detached {
                DetachedChoice::MoveCurrent => {
                    let tip = self.repo.read_ref(&game.id, &RefName::branch(&branch))?;
                    self.repo.update_ref(
                        lock,
                        &game.id,
                        &RefName::branch(&branch),
                        tip,
                        snap_id,
                        action,
                        &comment,
                    )?;
                }
                choice => {
                    let name = match choice {
                        DetachedChoice::NamedBranch(n) => n.clone(),
                        _ => self.unique_branch_name(&game.id, &branch)?,
                    };
                    self.repo.create_branch(lock, &game.id, &name, snap_id)?;
                    branch = name.clone();
                    new_branch = Some(name);
                }
            }
            self.repo.set_head(
                lock,
                &game.id,
                &Head {
                    branch: branch.clone(),
                    detached: None,
                },
            )?;
        } else {
            let tip = self.repo.read_ref(&game.id, &RefName::branch(&branch))?;
            self.repo.update_ref(
                lock,
                &game.id,
                &RefName::branch(&branch),
                tip,
                snap_id,
                action,
                &comment,
            )?;
        }
        if let Some(tag) = &opts.tag {
            self.repo.create_tag(lock, &game.id, tag, snap_id)?;
        }

        self.index.add_snapshot(&snap_id, &snap, &comment)?;
        self.index
            .set_refs(&game.id, &self.repo.list_refs(&game.id)?)?;
        self.write_index_from_tree(&game.id, &tree)?;

        Ok(BackupResult::Created {
            snapshot: snap_id,
            branch,
            new_branch,
            comment,
            summary,
            files: tree.entries.len() as u64,
            bytes: total,
            new_bytes,
            warnings,
        })
    }

    /// 读取一个文件入库；读取前后大小或 mtime 变化（游戏正在写入）时重试。
    /// 返回 (Blob ID, 大小, mtime, 新写入字节数)。
    fn store_file(&self, f: &DiskFile) -> Result<(ObjectId, u64, i64, u64)> {
        for _ in 0..STABLE_READ_ATTEMPTS {
            let file = std::fs::File::open(&f.abs).map_err(|e| AppError::io(&f.abs, e))?;
            let before = file.metadata().map_err(|e| AppError::io(&f.abs, e))?;
            let stats = self.repo.write_file(&file, &f.abs)?;
            drop(file);
            let after = std::fs::metadata(&f.abs).map_err(|e| AppError::io(&f.abs, e))?;
            let (m1, m2) = (scan::mtime_ns(&before), scan::mtime_ns(&after));
            if before.len() == after.len() && m1 == m2 && stats.size == before.len() {
                return Ok((stats.blob, stats.size, m1, stats.new_bytes));
            }
            tracing::warn!(path = %f.abs.display(), "文件在读取过程中被修改，重试");
        }
        Err(AppError::FileUnstable(f.abs.clone()))
    }

    /// 以清单内容整体替换变更检测缓存。
    pub(crate) fn write_index_from_tree(&mut self, game_id: &str, tree: &Tree) -> Result<()> {
        let now = scan::now_ns();
        let entries: Vec<IndexEntry> = tree
            .entries
            .iter()
            .map(|e| IndexEntry {
                path: e.path.clone(),
                size: e.size,
                mtime_ns: e.mtime_ns,
                hash: e.blob,
                indexed_at_ns: now,
            })
            .collect();
        self.index.replace_scan_entries(game_id, &entries)?;
        Ok(())
    }

    /// 内容未变、只有 mtime 变化的文件，刷新索引以免下次重算哈希。
    fn refresh_index(
        &mut self,
        game_id: &str,
        disk: &[DiskFile],
        changes: &[FileChange],
    ) -> Result<()> {
        let by_path: HashMap<&str, &DiskFile> = disk.iter().map(|f| (f.path.as_str(), f)).collect();
        let now = scan::now_ns();
        let entries: Vec<IndexEntry> = changes
            .iter()
            .filter(|c| c.refresh_index)
            .filter_map(|c| {
                let f = by_path.get(c.path.as_str())?;
                Some(IndexEntry {
                    path: c.path.clone(),
                    size: f.size,
                    mtime_ns: f.mtime_ns,
                    hash: c.hash?,
                    indexed_at_ns: now,
                })
            })
            .collect();
        if !entries.is_empty() {
            self.index.upsert_scan_entries(game_id, &entries)?;
        }
        Ok(())
    }

    fn auto_comment(&self, s: &ChangeSummary, first: bool, files: u64, bytes: u64) -> String {
        let l = self.locale;
        if first {
            return tf(
                l,
                "backup.auto.first",
                &[("files", &files), ("size", &human_bytes(bytes))],
            );
        }
        let mut parts = Vec::new();
        for (n, key) in [
            (s.added, "backup.auto.added"),
            (s.modified, "backup.auto.modified"),
            (s.deleted, "backup.auto.deleted"),
        ] {
            if n > 0 {
                parts.push(tf(l, key, &[("n", &n)]));
            }
        }
        if s.changed_bytes > 0 {
            parts.push(tf(
                l,
                "backup.auto.bytes",
                &[("size", &human_bytes(s.changed_bytes))],
            ));
        }
        let mut text = parts.join(t(l, "backup.auto.sep"));
        // 英文首字母大写。
        if let Some(first) = text.get(..1)
            && first.is_ascii()
        {
            text = first.to_ascii_uppercase() + &text[1..];
        }
        text
    }

    fn unique_branch_name(&self, game_id: &str, base: &str) -> Result<String> {
        let stem = format!("{base}-{}", now_compact());
        let existing: Vec<String> = self
            .repo
            .list_refs(game_id)?
            .into_iter()
            .map(|(r, _)| r.name.to_lowercase())
            .collect();
        let mut name = stem.clone();
        let mut i = 2;
        while existing.contains(&name.to_lowercase()) {
            name = format!("{stem}-{i}");
            i += 1;
        }
        Ok(name)
    }
}

pub fn check_comment(c: &str) -> Result<()> {
    let len = c.chars().count();
    if len > MAX_COMMENT_CHARS {
        return Err(AppError::CommentTooLong {
            len,
            max: MAX_COMMENT_CHARS,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Env;

    fn created(r: BackupResult) -> (ObjectId, String, ChangeSummary, u64, Option<String>) {
        match r {
            BackupResult::Created {
                snapshot,
                comment,
                summary,
                new_bytes,
                new_branch,
                ..
            } => (snapshot, comment, summary, new_bytes, new_branch),
            BackupResult::NoChanges => panic!("应生成快照"),
        }
    }

    fn backup(env: &mut Env, g: &GameRecord, opts: &BackupOptions) -> Result<BackupResult> {
        let lock = env.app.repo.lock().unwrap();
        env.app.backup(&lock, g, opts)
    }

    fn set_detached(env: &mut Env, g: &GameRecord, at: ObjectId) {
        let lock = env.app.repo.lock().unwrap();
        let branch = env.app.repo.head(&g.id).unwrap().branch;
        env.app
            .repo
            .set_head(
                &lock,
                &g.id,
                &Head {
                    branch,
                    detached: Some(at),
                },
            )
            .unwrap();
    }

    #[test]
    fn first_backup_then_no_changes_then_modify() {
        let mut env = Env::new();
        env.write("slot1.sav", b"level 1");
        env.write("子目录/存档 ã.sav", b"chapter 2");
        let g = env.add_game("空洞骑士");
        assert_eq!(env.app.status(&g).unwrap().state, GameState::NeverBackedUp);

        let (s1, comment, summary, _, _) =
            created(backup(&mut env, &g, &BackupOptions::default()).unwrap());
        assert!(comment.starts_with("首次备份：3 个文件"), "{comment}");
        assert_eq!(summary.added, 3, "两个存档 + 一个配置文件");
        let st = env.app.status(&g).unwrap();
        assert_eq!(st.state, GameState::UpToDate);
        assert_eq!(st.head, Some(s1));
        assert!(env.app.repo.root().join("games.toml").is_file());

        assert!(matches!(
            backup(&mut env, &g, &BackupOptions::default()).unwrap(),
            BackupResult::NoChanges
        ));

        env.write("slot1.sav", b"level 2!");
        assert_eq!(env.app.status(&g).unwrap().state, GameState::HasChanges);
        let (s2, comment, summary, _, _) =
            created(backup(&mut env, &g, &BackupOptions::default()).unwrap());
        assert_eq!(comment, "修改 1 个文件，+8 B");
        assert_eq!((summary.modified, summary.unchanged), (1, 2));
        let snap = env.app.repo.get_snapshot(&s2).unwrap();
        assert_eq!(snap.parents, vec![s1]);
        assert_eq!(snap.stats.changed, 1);
        assert_eq!(env.app.repo.resolve(&g.id, "main").unwrap(), s2);
        assert_eq!(env.app.index.snapshots_of(&g.id).unwrap().len(), 2);
    }

    #[test]
    fn deleted_and_added_files() {
        let mut env = Env::new();
        env.write("a", b"a");
        env.write("b", b"b");
        let g = env.add_game("G");
        backup(&mut env, &g, &BackupOptions::default()).unwrap();
        std::fs::remove_file(env.saves().join("a")).unwrap();
        env.write("c", b"cc");
        let (_, comment, s, _, _) =
            created(backup(&mut env, &g, &BackupOptions::default()).unwrap());
        assert_eq!((s.added, s.deleted), (1, 1));
        assert_eq!(comment, "新增 1 个文件，删除 1 个文件，+2 B");
    }

    #[test]
    fn english_auto_comment() {
        let mut env = Env::new();
        env.app.locale = crate::Locale::En;
        env.write("a", b"a");
        let g = env.add_game("G");
        backup(&mut env, &g, &BackupOptions::default()).unwrap();
        env.write("a", b"ab");
        let (_, comment, ..) = created(backup(&mut env, &g, &BackupOptions::default()).unwrap());
        assert_eq!(comment, "Modified 1 files, +2 B");
    }

    #[test]
    fn comment_tag_and_force() {
        let mut env = Env::new();
        env.write("a", b"a");
        let g = env.add_game("G");
        let long = BackupOptions {
            comment: Some("长".repeat(501)),
            ..Default::default()
        };
        assert!(matches!(
            backup(&mut env, &g, &long),
            Err(AppError::CommentTooLong { len: 501, .. })
        ));

        let opts = BackupOptions {
            comment: Some("Boss 前".into()),
            tag: Some("before-boss".into()),
            ..Default::default()
        };
        let (s1, comment, ..) = created(backup(&mut env, &g, &opts).unwrap());
        assert_eq!(comment, "Boss 前");
        assert_eq!(env.app.repo.resolve(&g.id, "before-boss").unwrap(), s1);
        assert_eq!(
            env.app
                .index
                .search_comments("Boss", Some(&g.id))
                .unwrap()
                .len(),
            1
        );

        let forced = BackupOptions {
            force: true,
            ..Default::default()
        };
        let (s2, comment, ..) = created(backup(&mut env, &g, &forced).unwrap());
        assert_eq!(comment, "手动快照（文件无变化）");
        assert_eq!(
            env.app.repo.get_snapshot(&s2).unwrap().tree,
            env.app.repo.get_snapshot(&s1).unwrap().tree
        );
    }

    #[test]
    fn unchanged_large_files_are_not_rewritten() {
        let mut env = Env::new();
        let big: Vec<u8> = (0..3_000_000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
            .collect();
        env.write("big.sav", &big);
        env.write("small.sav", b"1");
        let g = env.add_game("G");
        let (_, _, _, first_new, _) =
            created(backup(&mut env, &g, &BackupOptions::default()).unwrap());
        assert!(first_new >= big.len() as u64);
        env.write("small.sav", b"2");
        let (_, _, _, new_bytes, _) =
            created(backup(&mut env, &g, &BackupOptions::default()).unwrap());
        assert_eq!(new_bytes, 1, "只有改动的小文件写入仓库");
    }

    #[test]
    fn detached_head_creates_new_branch_by_default() {
        let mut env = Env::new();
        env.write("a", b"1");
        let g = env.add_game("G");
        let (s1, ..) = created(backup(&mut env, &g, &BackupOptions::default()).unwrap());
        env.write("a", b"2");
        let (s2, ..) = created(backup(&mut env, &g, &BackupOptions::default()).unwrap());

        set_detached(&mut env, &g, s1);
        env.write("a", b"3");
        let (s3, _, _, _, new_branch) =
            created(backup(&mut env, &g, &BackupOptions::default()).unwrap());
        let nb = new_branch.expect("应创建新分支");
        assert!(nb.starts_with("main-"), "{nb}");
        assert_eq!(
            env.app.repo.resolve(&g.id, "main").unwrap(),
            s2,
            "原主线不变"
        );
        assert_eq!(env.app.repo.resolve(&g.id, &nb).unwrap(), s3);
        assert_eq!(env.app.repo.get_snapshot(&s3).unwrap().parents, vec![s1]);
        assert_eq!(
            env.app.repo.head(&g.id).unwrap(),
            Head {
                branch: nb,
                detached: None
            }
        );
    }

    #[test]
    fn detached_head_move_current_or_named() {
        let mut env = Env::new();
        env.write("a", b"1");
        let g = env.add_game("G");
        let (s1, ..) = created(backup(&mut env, &g, &BackupOptions::default()).unwrap());
        env.write("a", b"2");
        backup(&mut env, &g, &BackupOptions::default()).unwrap();

        set_detached(&mut env, &g, s1);
        env.write("a", b"3");
        let named = BackupOptions {
            on_detached: DetachedChoice::NamedBranch("二周目".into()),
            ..Default::default()
        };
        let (s3, _, _, _, nb) = created(backup(&mut env, &g, &named).unwrap());
        assert_eq!(nb.as_deref(), Some("二周目"));
        assert_eq!(env.app.repo.resolve(&g.id, "二周目").unwrap(), s3);

        set_detached(&mut env, &g, s1);
        env.write("a", b"4");
        let opts = BackupOptions {
            on_detached: DetachedChoice::MoveCurrent,
            ..Default::default()
        };
        let (s4, _, _, _, nb) = created(backup(&mut env, &g, &opts).unwrap());
        assert!(nb.is_none());
        assert_eq!(env.app.repo.head(&g.id).unwrap().branch, "二周目");
        assert_eq!(env.app.repo.resolve(&g.id, "二周目").unwrap(), s4);
        assert_eq!(env.app.repo.resolve(&g.id, "二周目~1").unwrap(), s1);
    }

    #[test]
    fn config_files_follow_setting() {
        let mut env = Env::new();
        env.write("a", b"1");
        let g = env.add_game("G");
        env.app.config.backup.include_config_files = false;
        let (s1, ..) = created(backup(&mut env, &g, &BackupOptions::default()).unwrap());
        let tree = env
            .app
            .repo
            .get_tree(&env.app.repo.get_snapshot(&s1).unwrap().tree)
            .unwrap();
        assert_eq!(
            tree.entries
                .iter()
                .map(|e| e.path.as_str())
                .collect::<Vec<_>>(),
            ["0/a"]
        );
    }

    #[test]
    fn rebuilds_after_database_loss() {
        let mut env = Env::new();
        env.write("a", b"1");
        let g = env.add_game("黑暗之魂");
        let (s1, ..) = created(backup(&mut env, &g, &BackupOptions::default()).unwrap());
        let root = env.app.repo.root().to_path_buf();
        // 先换到别处的仓库，释放数据库文件。
        env.app = crate::testutil::open(&env.dir.path().join("elsewhere"));
        for f in ["bonfire.db", "bonfire.db-wal", "bonfire.db-shm"] {
            let _ = std::fs::remove_file(root.join(f));
        }
        env.reopen();
        let g2 = env.app.find_game("黑暗之魂").unwrap();
        assert_eq!(g2, g);
        let st = env.app.status(&g2).unwrap();
        assert_eq!(st.state, GameState::UpToDate);
        assert_eq!(st.head, Some(s1));
    }

    #[test]
    fn repo_cannot_share_dir_with_app_config() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::AppPaths::under_root(dir.path(), false);
        let r = crate::App::open(
            paths,
            crate::Config::default(),
            crate::Locale::ZhCn,
            dir.path(),
        );
        assert!(matches!(r, Err(AppError::Invalid(_))));
    }

    #[test]
    fn find_game_by_name_and_remove() {
        let mut env = Env::new();
        let g = env.add_game("Hollow Knight");
        assert_eq!(env.app.find_game("hollow knight").unwrap().id, g.id);
        assert!(matches!(
            env.app.find_game("nope"),
            Err(AppError::GameNotFound(_))
        ));
        let lock = env.app.repo.lock().unwrap();
        env.app.remove_game(&lock, &g.id).unwrap();
        assert!(env.app.games().unwrap().is_empty());
    }
}

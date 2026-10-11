//! 恢复与撤销（文档 8.2）。
//!
//! 流程：生成计划（预览）→ 游戏运行检测 → 当前状态与最新快照不同时先建保护快照 →
//! 全部文件先写到同目录的临时文件 → 逐个原子替换 → 失败则按恢复前的状态回滚 →
//! 记录操作，完全恢复时 HEAD 标记为“已从 X 恢复”。
//! 只有本模块写入存档目录。

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use bonfire_core::fault;
use bonfire_core::index::{GameRecord, IndexEntry};
use bonfire_core::refs::{Head, RefKind};
use bonfire_core::scan::{self, hash_file};
use bonfire_core::{ObjectId, RepoLock, Tree, TreeEntry, Trigger};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::backup::{BackupOptions, BackupResult};
use crate::error::{AppError, Result};
use crate::i18n::tf;

/// 故障注入点：已替换部分文件。
pub const FAULT_MID_COMMIT: &str = "restore-mid-commit";
const TMP_SUFFIX: &str = ".bonfire-restore.tmp";

#[derive(Debug, Clone, Default)]
pub struct RestoreOptions {
    /// 只恢复匹配的文件（glob，匹配清单路径或去掉位置序号后的相对路径）。
    pub paths: Vec<String>,
    /// 完全恢复：同时删除目标快照中没有的文件。
    pub full: bool,
    /// 只生成计划，不写入。
    pub dry_run: bool,
    /// 忽略游戏运行检测。
    pub force: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Overwrite,
    Create,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedFile {
    pub path: String,
    pub disk: PathBuf,
    pub action: Action,
    pub size: u64,
    #[serde(skip)]
    blob: Option<ObjectId>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePlan {
    pub target: ObjectId,
    pub files: Vec<PlannedFile>,
    pub unchanged: u64,
    /// 只恢复了部分文件（指定了路径过滤）。
    pub partial: bool,
    /// 检测到正在运行的游戏进程。
    pub running: Vec<String>,
}

impl RestorePlan {
    pub fn count(&self, a: Action) -> usize {
        self.files.iter().filter(|f| f.action == a).count()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    pub plan: RestorePlan,
    /// 恢复前自动生成的保护快照。
    pub protection: Option<ObjectId>,
    pub dry_run: bool,
}

/// 操作日志中记录的恢复详情，供撤销使用。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestoreDetail {
    target: ObjectId,
    /// 恢复前的状态对应的快照（保护快照或当时的最新快照）。
    previous: ObjectId,
    protection: Option<ObjectId>,
    previous_head: Head,
    partial: bool,
    files: usize,
}

impl App {
    /// 生成恢复计划（预览差异）。
    pub fn plan_restore(
        &self,
        game: &GameRecord,
        target_spec: &str,
        opts: &RestoreOptions,
    ) -> Result<RestorePlan> {
        let target = self.repo.resolve(&game.id, target_spec)?;
        self.plan_for(game, target, opts)
    }

    fn plan_for(
        &self,
        game: &GameRecord,
        target: ObjectId,
        opts: &RestoreOptions,
    ) -> Result<RestorePlan> {
        let tree = self.repo.get_tree(&self.repo.get_snapshot(&target)?.tree)?;
        let filter = build_filter(&opts.paths)?;
        let selected = |p: &str| {
            filter.as_ref().is_none_or(|f| {
                f.is_match(p) || p.split_once('/').is_some_and(|(_, rel)| f.is_match(rel))
            })
        };

        let mut files = Vec::new();
        let mut unchanged = 0;
        for e in tree.entries.iter().filter(|e| selected(&e.path)) {
            let disk = self.disk_path(&game.id, &e.path)?;
            let action = match std::fs::metadata(&disk) {
                Ok(m) if m.is_file() && m.len() == e.size && hash_file(&disk)? == e.blob => None,
                Ok(_) => Some(Action::Overwrite),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Some(Action::Create),
                Err(err) => return Err(AppError::io(&disk, err)),
            };
            match action {
                Some(action) => files.push(PlannedFile {
                    path: e.path.clone(),
                    disk,
                    action,
                    size: e.size,
                    blob: Some(e.blob),
                }),
                None => unchanged += 1,
            }
        }
        if opts.full {
            let in_target: std::collections::HashSet<&str> =
                tree.entries.iter().map(|e| e.path.as_str()).collect();
            for f in scan::collect_files(&self.scan_roots(game)?)? {
                if !in_target.contains(f.path.as_str()) && selected(&f.path) {
                    files.push(PlannedFile {
                        path: f.path,
                        disk: f.abs,
                        action: Action::Delete,
                        size: f.size,
                        blob: None,
                    });
                }
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let running = crate::process::running(&[self.watch_of(game)?])
            .into_iter()
            .flat_map(|(_, names)| names)
            .collect();
        Ok(RestorePlan {
            target,
            files,
            unchanged,
            partial: filter.is_some(),
            running,
        })
    }

    /// 恢复到指定快照。
    pub(crate) fn restore_raw(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        target_spec: &str,
        opts: &RestoreOptions,
    ) -> Result<RestoreReport> {
        let target = self.repo.resolve(&game.id, target_spec)?;
        self.restore_to(lock, game, target, opts, "restore")
    }

    fn restore_to(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        target: ObjectId,
        opts: &RestoreOptions,
        kind: &str,
    ) -> Result<RestoreReport> {
        let plan = self.plan_for(game, target, opts)?;
        if !plan.running.is_empty() && !opts.force && !opts.dry_run {
            return Err(AppError::GameRunning {
                processes: plan.running.join("、"),
            });
        }
        if opts.dry_run {
            return Ok(RestoreReport {
                plan,
                protection: None,
                dry_run: true,
            });
        }

        // 保护快照：当前状态与最新快照不同（或从未备份）时先入库，保证恢复不丢数据。
        let status = self.status(game)?;
        let mut protection = None;
        if status.head.is_none() || status.summary.has_changes() {
            let comment = tf(
                self.locale,
                "restore.auto.protect",
                &[("target", &target.short(12))],
            );
            let opts = BackupOptions {
                comment: Some(comment),
                trigger: Trigger::PreRestore,
                force: true,
                ..Default::default()
            };
            if let BackupResult::Created { snapshot, .. } = self.backup(lock, game, &opts)? {
                protection = Some(snapshot);
            }
        }
        let previous = match protection.or(self.repo.head_snapshot(&game.id)?) {
            Some(p) => p,
            None => return Err(AppError::Invalid("无法确定恢复前的状态".into())),
        };
        let previous_head = self.repo.head(&game.id)?;
        let detail = RestoreDetail {
            target,
            previous,
            protection,
            previous_head: previous_head.clone(),
            partial: plan.partial,
            files: plan.files.len(),
        };
        let op = self.index.begin_op(
            kind,
            Some(&game.id),
            &serde_json::to_value(&detail).unwrap(),
        )?;

        if let Err(e) = self.apply_plan(game, &plan, previous) {
            self.index.finish_op(
                op,
                "failed",
                &serde_json::json!({ "error": e.to_string(), "detail": detail }),
            )?;
            return Err(e);
        }

        if !plan.partial {
            // 完全按目标快照恢复：HEAD 与变更检测缓存都对应目标快照。
            // 目标是某个分支的最新快照时接回这个分支（当前分支优先），之后的备份继续记在这个分支上；
            // 否则为“已从旧快照恢复”，下次备份时按选择新建或移动分支。
            let tips: Vec<String> = self
                .repo
                .list_refs(&game.id)?
                .into_iter()
                .filter(|(r, id)| r.kind == RefKind::Branch && *id == target)
                .map(|(r, _)| r.name)
                .collect();
            let head = if tips.contains(&previous_head.branch) {
                Head {
                    branch: previous_head.branch.clone(),
                    detached: None,
                }
            } else if let [only] = tips.as_slice() {
                Head {
                    branch: only.clone(),
                    detached: None,
                }
            } else {
                Head {
                    branch: previous_head.branch.clone(),
                    detached: Some(target),
                }
            };
            self.repo.set_head(lock, &game.id, &head)?;
            self.reindex_from_disk(
                game,
                &self.repo.get_tree(&self.repo.get_snapshot(&target)?.tree)?,
            )?;
        }
        self.index
            .finish_op(op, "done", &serde_json::to_value(&detail).unwrap())?;
        Ok(RestoreReport {
            plan,
            protection,
            dry_run: false,
        })
    }

    /// 撤销最近一次恢复：恢复到恢复前的状态，并还原 HEAD。仅当之后没有再备份时可用。
    pub(crate) fn undo_restore_raw(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
    ) -> Result<RestoreReport> {
        let op = self
            .index
            .last_op(&game.id)?
            .ok_or_else(|| AppError::NothingToUndo(game.name.clone()))?;
        let undoable = op.kind == "restore" && matches!(op.status.as_str(), "done" | "running");
        if !undoable {
            return Err(AppError::NothingToUndo(game.name.clone()));
        }
        let detail: RestoreDetail = serde_json::from_value(op.detail.clone())
            .map_err(|e| AppError::Invalid(format!("操作记录格式错误：{e}")))?;
        let opts = RestoreOptions {
            full: true,
            force: true,
            ..Default::default()
        };
        let report = self.restore_to(lock, game, detail.previous, &opts, "undo-restore")?;
        self.repo.set_head(lock, &game.id, &detail.previous_head)?;
        self.index.set_op_status(op.id, "undone")?;
        Ok(report)
    }

    /// 可撤销的恢复操作用到的快照（清理历史时保留）。
    pub(crate) fn undo_snapshots(&self, game_id: &str) -> Result<Vec<ObjectId>> {
        let Some((_, d)) = self.undoable_restore(game_id)? else {
            return Ok(Vec::new());
        };
        let mut out = vec![d.target, d.previous];
        out.extend(d.protection);
        out.extend(d.previous_head.detached);
        Ok(out)
    }

    /// 快照因删除前面的快照而改了 ID 时，同步改写可撤销的恢复操作记录。
    pub(crate) fn remap_undo(
        &self,
        game_id: &str,
        map: &HashMap<ObjectId, ObjectId>,
    ) -> Result<()> {
        let Some((op, mut d)) = self.undoable_restore(game_id)? else {
            return Ok(());
        };
        let m = |id: &mut ObjectId| {
            if let Some(n) = map.get(id) {
                *id = *n;
            }
        };
        m(&mut d.target);
        m(&mut d.previous);
        if let Some(p) = d.protection.as_mut() {
            m(p);
        }
        if let Some(p) = d.previous_head.detached.as_mut() {
            m(p);
        }
        let detail = serde_json::to_value(&d).expect("操作记录序列化不会失败");
        if detail != op.detail {
            self.index.set_op_detail(op.id, &detail)?;
        }
        Ok(())
    }

    fn undoable_restore(
        &self,
        game_id: &str,
    ) -> Result<Option<(bonfire_core::index::OperationRow, RestoreDetail)>> {
        let Some(op) = self.index.last_op(game_id)? else {
            return Ok(None);
        };
        if op.kind != "restore" || !matches!(op.status.as_str(), "done" | "running") {
            return Ok(None);
        }
        Ok(serde_json::from_value(op.detail.clone())
            .ok()
            .map(|d| (op, d)))
    }

    /// 写入计划中的文件。任何一步失败都回滚到 `previous` 快照的内容。
    fn apply_plan(&self, game: &GameRecord, plan: &RestorePlan, previous: ObjectId) -> Result<()> {
        // 1. 全部写到临时文件；此阶段失败不影响存档。
        let mut staged: Vec<(&PlannedFile, PathBuf)> = Vec::new();
        let stage = (|| -> Result<()> {
            for f in plan.files.iter().filter(|f| f.action != Action::Delete) {
                let tmp = tmp_path(&f.disk);
                write_blob(self, f.blob.expect("写入项有 Blob"), &tmp)?;
                staged.push((f, tmp));
            }
            Ok(())
        })();
        if let Err(e) = stage {
            for (_, tmp) in &staged {
                let _ = std::fs::remove_file(tmp);
            }
            return Err(e);
        }

        // 2. 逐个替换，再删除多余文件。
        let mut touched: Vec<&PlannedFile> = Vec::new();
        let commit = (|| -> Result<()> {
            for (f, tmp) in &staged {
                replace(tmp, &f.disk)?;
                touched.push(f);
                fault::point(FAULT_MID_COMMIT);
            }
            for f in plan.files.iter().filter(|f| f.action == Action::Delete) {
                clear_readonly(&f.disk);
                std::fs::remove_file(&f.disk).map_err(|e| AppError::io(&f.disk, e))?;
                touched.push(f);
            }
            Ok(())
        })();
        if let Err(e) = commit {
            for (_, tmp) in &staged {
                let _ = std::fs::remove_file(tmp);
            }
            return match self.rollback(game, &touched, previous) {
                Ok(()) => Err(AppError::RolledBack(e.to_string())),
                Err(re) => Err(AppError::Invalid(format!(
                    "恢复失败（{e}），回滚也失败（{re}）；恢复前的状态保存在快照 {previous} 中"
                ))),
            };
        }

        // 3. 恢复的文件 mtime 设为当前时间，使平台云存档判定本地较新（文档 10.3）。
        let now = std::time::SystemTime::now();
        for (f, _) in &staged {
            if let Ok(file) = std::fs::File::options().write(true).open(&f.disk) {
                let _ = file.set_modified(now);
            }
        }
        // 4. 注册表存档：把恢复出的 .reg 导入注册表。
        for (f, _) in &staged {
            self.import_registry(&f.disk)?;
        }
        Ok(())
    }

    /// 恢复出的文件是注册表导出文件时执行 reg import。
    fn import_registry(&self, disk: &Path) -> Result<()> {
        if disk.starts_with(self.paths.cache_dir.join("registry")) {
            bonfire_discovery::registry::import(disk).map_err(|e| AppError::io(disk, e))?;
        }
        Ok(())
    }

    /// 把已改动的文件还原为 `previous` 快照中的内容；该快照中没有的文件删除。
    fn rollback(
        &self,
        game: &GameRecord,
        touched: &[&PlannedFile],
        previous: ObjectId,
    ) -> Result<()> {
        let tree = self
            .repo
            .get_tree(&self.repo.get_snapshot(&previous)?.tree)?;
        let by_path: HashMap<&str, &TreeEntry> =
            tree.entries.iter().map(|e| (e.path.as_str(), e)).collect();
        for f in touched {
            match by_path.get(f.path.as_str()) {
                Some(e) => {
                    let disk = self.disk_path(&game.id, &e.path)?;
                    let tmp = tmp_path(&disk);
                    write_blob(self, e.blob, &tmp)?;
                    replace(&tmp, &disk)?;
                    self.import_registry(&disk)?;
                }
                None => {
                    let _ = std::fs::remove_file(&f.disk);
                }
            }
        }
        Ok(())
    }

    /// 完全恢复后，以目标清单为准、磁盘上的实际大小与 mtime 写入变更检测缓存。
    fn reindex_from_disk(&mut self, game: &GameRecord, tree: &Tree) -> Result<()> {
        let now = scan::now_ns();
        let mut entries = Vec::with_capacity(tree.entries.len());
        for e in &tree.entries {
            let disk = self.disk_path(&game.id, &e.path)?;
            let m = std::fs::metadata(&disk).map_err(|err| AppError::io(&disk, err))?;
            entries.push(IndexEntry {
                path: e.path.clone(),
                size: m.len(),
                mtime_ns: scan::mtime_ns(&m),
                hash: e.blob,
                indexed_at_ns: now,
            });
        }
        self.index.replace_scan_entries(&game.id, &entries)?;
        Ok(())
    }
}

fn build_filter(patterns: &[String]) -> Result<Option<GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        let p = p.replace('\\', "/");
        b.add(Glob::new(&p).map_err(|e| AppError::Invalid(format!("无效的路径模式 {p}：{e}")))?);
    }
    Ok(Some(
        b.build().map_err(|e| AppError::Invalid(e.to_string()))?,
    ))
}

fn tmp_path(disk: &Path) -> PathBuf {
    let mut name = disk
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(TMP_SUFFIX);
    disk.with_file_name(name)
}

fn write_blob(app: &App, blob: ObjectId, dest: &Path) -> Result<()> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
    }
    let mut f = std::fs::File::create(dest).map_err(|e| AppError::io(dest, e))?;
    app.repo.read_blob_to(&blob, &mut f, dest)?;
    f.sync_all().map_err(|e| AppError::io(dest, e))
}

fn clear_readonly(p: &Path) {
    if let Ok(m) = std::fs::metadata(p) {
        let mut perm = m.permissions();
        if perm.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perm.set_readonly(false);
            let _ = std::fs::set_permissions(p, perm);
        }
    }
}

/// 原子替换目标文件（同目录重命名）。
fn replace(tmp: &Path, dest: &Path) -> Result<()> {
    clear_readonly(dest);
    std::fs::rename(tmp, dest).map_err(|e| AppError::io(dest, e))
}

/// 恢复计划的汇总，供界面与 CLI 显示。
pub fn plan_summary(plan: &RestorePlan) -> BTreeMap<&'static str, usize> {
    BTreeMap::from([
        ("overwrite", plan.count(Action::Overwrite)),
        ("create", plan.count(Action::Create)),
        ("delete", plan.count(Action::Delete)),
        ("unchanged", plan.unchanged as usize),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GameState;
    use crate::testutil::Env;

    fn backup(env: &mut Env, g: &GameRecord) -> ObjectId {
        let lock = env.app.repo.lock().unwrap();
        match env.app.backup(&lock, g, &BackupOptions::default()).unwrap() {
            BackupResult::Created { snapshot, .. } => snapshot,
            BackupResult::NoChanges => panic!("应生成快照"),
        }
    }

    fn restore(
        env: &mut Env,
        g: &GameRecord,
        spec: &str,
        opts: &RestoreOptions,
    ) -> Result<RestoreReport> {
        let lock = env.app.repo.lock().unwrap();
        env.app.restore(&lock, g, spec, opts)
    }

    fn undo(env: &mut Env, g: &GameRecord) -> Result<RestoreReport> {
        let lock = env.app.repo.lock().unwrap();
        env.app.undo_restore(&lock, g)
    }

    /// v1: a=1 b=1；v2: a=2 b=1 c=2
    fn two_versions(env: &mut Env) -> (GameRecord, ObjectId, ObjectId) {
        env.write("a.sav", b"a1");
        env.write("sub/b.sav", b"b1");
        let g = env.add_game("G");
        let s1 = backup(env, &g);
        env.write("a.sav", b"a2");
        env.write("c.sav", b"c2");
        let s2 = backup(env, &g);
        (g, s1, s2)
    }

    #[test]
    fn restoring_a_branch_tip_switches_to_that_branch() {
        let mut env = Env::new();
        let (g, s1, s2) = two_versions(&mut env);
        {
            let lock = env.app.repo.lock().unwrap();
            env.app
                .create_ref(&lock, &g, RefKind::Branch, "side", Some(&s1.to_string()))
                .unwrap();
        }
        restore(&mut env, &g, "side", &RestoreOptions::default()).unwrap();
        let head = env.app.repo.head(&g.id).unwrap();
        assert_eq!((head.branch.as_str(), head.detached), ("side", None));
        // 之后的备份继续记在 side 上，不另建分支。
        env.write("a.sav", b"a3");
        let s3 = backup(&mut env, &g);
        assert_eq!(env.app.repo.resolve(&g.id, "side").unwrap(), s3);
        assert_eq!(env.app.repo.get_snapshot(&s3).unwrap().parents, vec![s1]);
        // 回到 main 的最新快照：接回 main。
        restore(&mut env, &g, "main", &RestoreOptions::default()).unwrap();
        let head = env.app.repo.head(&g.id).unwrap();
        assert_eq!((head.branch.as_str(), head.detached), ("main", None));
        assert_eq!(env.app.repo.resolve(&g.id, "HEAD").unwrap(), s2);
    }

    #[test]
    fn restore_old_snapshot_and_head_detaches() {
        let mut env = Env::new();
        let (g, s1, s2) = two_versions(&mut env);
        let r = restore(&mut env, &g, "HEAD~1", &RestoreOptions::default()).unwrap();
        assert_eq!(r.protection, None, "磁盘与最新快照一致时不需要保护快照");
        assert_eq!(
            (
                r.plan.count(Action::Overwrite),
                r.plan.count(Action::Delete),
                r.plan.unchanged
            ),
            (1, 0, 2)
        );
        assert_eq!(env.read("a.sav").unwrap(), b"a1");
        assert_eq!(env.read("c.sav").unwrap(), b"c2", "默认只覆盖不删除");
        let head = env.app.repo.head(&g.id).unwrap();
        assert_eq!(head.detached, Some(s1));
        assert_eq!(
            env.app.repo.resolve(&g.id, "main").unwrap(),
            s2,
            "分支不移动"
        );
        // 只有 c.sav 是快照外的文件，显示为新增。
        let st = env.app.status(&g).unwrap();
        assert_eq!(
            (st.state, st.summary.added, st.summary.modified),
            (GameState::HasChanges, 1, 0)
        );
    }

    #[test]
    fn full_restore_deletes_extra_files_and_is_up_to_date() {
        let mut env = Env::new();
        let (g, s1, _) = two_versions(&mut env);
        let opts = RestoreOptions {
            full: true,
            ..Default::default()
        };
        let r = restore(&mut env, &g, &s1.short(8), &opts).unwrap();
        assert_eq!(r.plan.count(Action::Delete), 1);
        assert!(env.read("c.sav").is_none());
        assert_eq!(env.app.status(&g).unwrap().state, GameState::UpToDate);
    }

    #[test]
    fn protection_snapshot_and_undo() {
        let mut env = Env::new();
        let (g, s1, s2) = two_versions(&mut env);
        env.write("a.sav", b"unsaved progress");
        let r = restore(
            &mut env,
            &g,
            &s1.to_string(),
            &RestoreOptions {
                full: true,
                ..Default::default()
            },
        )
        .unwrap();
        let p = r.protection.expect("有未备份的改动，应生成保护快照");
        let snap = env.app.repo.get_snapshot(&p).unwrap();
        assert_eq!(snap.trigger, Trigger::PreRestore);
        assert_eq!(snap.parents, vec![s2]);
        assert!(snap.comment.contains(&s1.short(12)), "{}", snap.comment);
        assert_eq!(env.read("a.sav").unwrap(), b"a1");

        undo(&mut env, &g).unwrap();
        assert_eq!(env.read("a.sav").unwrap(), b"unsaved progress");
        assert_eq!(env.read("c.sav").unwrap(), b"c2");
        assert_eq!(env.app.repo.head(&g.id).unwrap().detached, None);
        assert_eq!(env.app.repo.resolve(&g.id, "HEAD").unwrap(), p);
        assert_eq!(env.app.status(&g).unwrap().state, GameState::UpToDate);
        assert!(
            matches!(undo(&mut env, &g), Err(AppError::NothingToUndo(_))),
            "不能连续撤销"
        );
    }

    #[test]
    fn undo_not_available_after_backup() {
        let mut env = Env::new();
        let (g, s1, _) = two_versions(&mut env);
        restore(&mut env, &g, &s1.to_string(), &RestoreOptions::default()).unwrap();
        env.write("a.sav", b"played after restore");
        backup(&mut env, &g);
        assert!(matches!(
            undo(&mut env, &g),
            Err(AppError::NothingToUndo(_))
        ));
    }

    #[test]
    fn partial_restore_keeps_head() {
        let mut env = Env::new();
        let (g, s1, _) = two_versions(&mut env);
        env.write("sub/b.sav", b"b-new");
        let opts = RestoreOptions {
            paths: vec!["a.sav".into()],
            ..Default::default()
        };
        let r = restore(&mut env, &g, &s1.to_string(), &opts).unwrap();
        assert!(r.plan.partial);
        assert_eq!(r.plan.files.len(), 1);
        assert_eq!(env.read("a.sav").unwrap(), b"a1");
        assert_eq!(env.read("sub/b.sav").unwrap(), b"b-new", "未选中的文件不动");
        assert_eq!(env.app.repo.head(&g.id).unwrap().detached, None);
        // 通配也能匹配带位置序号的清单路径。
        let opts = RestoreOptions {
            paths: vec!["0/sub/*.sav".into()],
            dry_run: true,
            ..Default::default()
        };
        assert_eq!(
            restore(&mut env, &g, &s1.to_string(), &opts)
                .unwrap()
                .plan
                .files
                .len(),
            1
        );
    }

    #[test]
    fn dry_run_changes_nothing() {
        let mut env = Env::new();
        let (g, s1, _) = two_versions(&mut env);
        let r = restore(
            &mut env,
            &g,
            &s1.to_string(),
            &RestoreOptions {
                dry_run: true,
                full: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(r.dry_run);
        assert_eq!(plan_summary(&r.plan)["overwrite"], 1);
        assert_eq!(plan_summary(&r.plan)["delete"], 1);
        assert_eq!(env.read("a.sav").unwrap(), b"a2");
        assert!(env.app.index.last_op(&g.id).unwrap().unwrap().kind != "restore");
    }

    #[test]
    fn failure_mid_commit_rolls_back() {
        let mut env = Env::new();
        env.write("a.sav", b"a1");
        env.write("z.sav", b"z1");
        let g = env.add_game("G");
        let s1 = backup(&mut env, &g);
        env.write("a.sav", b"a2");
        env.write("z.sav", b"z2");
        backup(&mut env, &g);
        // z.sav 被替换成同名目录：替换 a.sav 成功后在 z.sav 处失败。
        std::fs::remove_file(env.saves().join("z.sav")).unwrap();
        std::fs::create_dir_all(env.saves().join("z.sav/inner")).unwrap();
        let err = restore(
            &mut env,
            &g,
            &s1.to_string(),
            &RestoreOptions {
                force: true,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, AppError::RolledBack(_)), "{err}");
        assert_eq!(
            env.read("a.sav").unwrap(),
            b"a2",
            "已替换的文件回滚为恢复前的内容"
        );
        let leftovers: Vec<_> = walk(&env.saves())
            .into_iter()
            .filter(|p| p.to_string_lossy().ends_with(TMP_SUFFIX))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        assert_eq!(
            env.app.index.last_op(&g.id).unwrap().unwrap().status,
            "failed"
        );
    }

    #[test]
    fn overwrites_read_only_file() {
        let mut env = Env::new();
        let (g, s1, _) = two_versions(&mut env);
        let p = env.saves().join("a.sav");
        let mut perm = std::fs::metadata(&p).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&p, perm).unwrap();
        restore(&mut env, &g, &s1.to_string(), &RestoreOptions::default()).unwrap();
        assert_eq!(env.read("a.sav").unwrap(), b"a1");
    }

    #[test]
    #[cfg(windows)]
    fn running_game_blocks_restore_unless_forced() {
        let mut env = Env::new();
        let install = env.dir.path().join("install");
        std::fs::create_dir_all(&install).unwrap();
        env.write("a.sav", b"a1");
        let lock = env.app.repo.lock().unwrap();
        let new = crate::NewGame {
            name: "G".into(),
            save_paths: vec![env.saves()],
            config_paths: vec![],
            install_dir: Some(install.clone()),
        };
        let g = env.app.add_game(&lock, &new).unwrap();
        drop(lock);
        let s1 = backup(&mut env, &g);
        env.write("a.sav", b"a2");
        backup(&mut env, &g);

        let exe = install.join("Game.exe");
        std::fs::copy(r"C:\Windows\System32\ping.exe", &exe).unwrap();
        let mut child = std::process::Command::new(&exe)
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let blocked = restore(&mut env, &g, &s1.to_string(), &RestoreOptions::default());
        let forced = restore(
            &mut env,
            &g,
            &s1.to_string(),
            &RestoreOptions {
                force: true,
                ..Default::default()
            },
        );
        child.kill().unwrap();
        let _ = child.wait();
        assert!(
            matches!(blocked, Err(AppError::GameRunning { .. })),
            "{blocked:?}"
        );
        forced.unwrap();
        assert_eq!(env.read("a.sav").unwrap(), b"a1");
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            if e.path().is_dir() {
                out.extend(walk(&e.path()));
            } else {
                out.push(e.path());
            }
        }
        out
    }
}

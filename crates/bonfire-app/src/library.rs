//! 界面用的视图模型：游戏库列表的一行、完整的历史图、跨游戏的备注搜索结果。

use std::collections::HashMap;

use bonfire_core::index::{GameRecord, LocationRecord};
use bonfire_core::refs::RefKind;
use bonfire_core::scan::{ChangeSummary, FileStatus};
use bonfire_core::{ObjectId, TreeEntry};
use serde::Serialize;

use crate::app::App;
use crate::backup::GameState;
use crate::error::Result;

/// 游戏库中的一行（文档 9.1 游戏库：封面、名称、平台、状态、变化文件数、最后备份时间）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameRow {
    pub id: String,
    pub name: String,
    pub name_zh: Option<String>,
    pub platform: String,
    pub enabled: bool,
    pub cover_thumb: Option<String>,
    pub state: GameState,
    pub summary: ChangeSummary,
    pub files: u64,
    pub total_bytes: u64,
    pub last_modified_ns: Option<i64>,
    pub head: Option<ObjectId>,
    pub branch: String,
    pub detached: bool,
    pub snapshots: u64,
    pub last_backup_at: Option<String>,
    pub cloud_save: bool,
    pub playtime_min: Option<u64>,
    /// 待确认的推测存档位置数（状态徽标“待确认”）。
    pub unconfirmed: u64,
    /// 从别处导入、本机还没有安装（或还没扫描到）的游戏。
    pub installed: bool,
    /// 本机还不能确定路径的存档位置数（未安装的游戏需要安装目录的位置）。
    pub unresolved: u64,
    /// 相册中的截图数（不含回收站）。
    pub shots: u64,
    /// 只管理截图（不备份存档）。
    pub shots_only: bool,
    /// 管理方式：存档和截图 / 只管理存档 / 只管理截图。
    pub mode: crate::shots::ManageMode,
    /// 磁盘上还没有收集的新截图数。
    pub new_shots: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryNode {
    pub id: ObjectId,
    pub parents: Vec<ObjectId>,
    pub created_at: String,
    pub trigger: String,
    pub comment: String,
    pub files: u64,
    pub bytes: u64,
    pub changed: u64,
    pub playtime_min: Option<u64>,
    pub host: String,
    /// 指向该快照的分支与标签，如 `heads/main`。
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefView {
    pub name: String,
    pub kind: RefKind,
    pub target: ObjectId,
    pub current: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryView {
    pub game: GameRecord,
    pub head: Option<ObjectId>,
    pub branch: String,
    pub detached: bool,
    pub refs: Vec<RefView>,
    /// 全部快照，最新的在前。
    pub nodes: Vec<HistoryNode>,
    pub locations: Vec<LocationRecord>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub game_id: String,
    pub game_name: String,
    pub id: ObjectId,
    pub comment: String,
    pub created_at: String,
}

impl App {
    pub fn game_row(&self, game: &GameRecord, force_verify: bool) -> Result<GameRow> {
        let st = if force_verify {
            self.status_verified(game)?
        } else {
            self.status(game)?
        };
        let snaps = self.index.snapshots_of(&game.id)?;
        let meta = self.metadata(&game.id)?;
        let locations = self.locations(&game.id)?;
        let unconfirmed = locations
            .iter()
            .filter(|l| !l.confirmed && !l.resolved_path.is_empty())
            .count() as u64;
        let unresolved = locations
            .iter()
            .filter(|l| l.resolved_path.is_empty())
            .count() as u64;
        let installed =
            game.install_dir.is_some() || !locations.iter().any(|l| l.source == "import");
        let name_zh = meta.as_ref().and_then(|m| m.data.name_zh.clone());
        Ok(GameRow {
            id: game.id.clone(),
            name: game.name.clone(),
            name_zh,
            platform: game.platform.clone(),
            enabled: game.enabled,
            cover_thumb: meta.as_ref().and_then(|m| {
                m.cover_thumb_path
                    .clone()
                    .or_else(|| m.data.cover_path.clone())
            }),
            state: st.state,
            summary: st.summary,
            files: st.files,
            total_bytes: st.total_bytes,
            last_modified_ns: st.last_modified_ns,
            head: st.head,
            branch: st.branch,
            detached: st.detached,
            snapshots: snaps.len() as u64,
            last_backup_at: snaps.iter().map(|s| s.created_at.clone()).max(),
            cloud_save: meta
                .as_ref()
                .and_then(|m| m.data.cloud_save)
                .unwrap_or(false),
            playtime_min: meta.as_ref().and_then(|m| m.data.playtime_min),
            unconfirmed,
            installed,
            unresolved,
            shots: self.shot_count(&game.id)?,
            shots_only: st.state == GameState::ShotsOnly,
            mode: self.manage_mode(game)?,
            // 截图来源读取失败不影响游戏库。
            new_shots: self.pending_shots(game).unwrap_or(0) as u64,
        })
    }

    /// 变化的文件（不含未变化的）。
    pub fn changed_files(&self, game: &GameRecord) -> Result<Vec<bonfire_core::scan::FileChange>> {
        Ok(self
            .status(game)?
            .changes
            .into_iter()
            .filter(|c| c.status != FileStatus::Unchanged)
            .collect())
    }

    /// 完整历史：所有快照及其父子关系、分支与标签。
    pub fn history_view(&self, game: &GameRecord) -> Result<HistoryView> {
        let head = self.repo.head(&game.id)?;
        let head_snap = self.repo.head_snapshot(&game.id)?;
        let mut labels: HashMap<ObjectId, Vec<String>> = HashMap::new();
        let mut refs = Vec::new();
        for (r, id) in self.repo.list_refs(&game.id)? {
            labels.entry(id).or_default().push(r.to_string());
            let current =
                r.kind == RefKind::Branch && r.name == head.branch && head.detached.is_none();
            refs.push(RefView {
                name: r.name.clone(),
                kind: r.kind,
                target: id,
                current,
            });
        }
        let mut nodes = Vec::new();
        for row in self.index.snapshots_of(&game.id)? {
            let s = self.repo.get_snapshot(&row.id)?;
            nodes.push(HistoryNode {
                id: row.id,
                parents: s.parents.clone(),
                created_at: s.created_at.clone(),
                trigger: row.trigger.clone(),
                comment: row.comment.clone(),
                files: s.stats.files,
                bytes: s.stats.bytes,
                changed: s.stats.changed,
                playtime_min: s.playtime_min,
                host: s.host.clone(),
                refs: labels.remove(&row.id).unwrap_or_default(),
            });
        }
        // 时间精确到秒：同一秒内的快照按代数（父在前、子在后）排序，保证顺序确定。
        let by_id: HashMap<ObjectId, Vec<ObjectId>> =
            nodes.iter().map(|n| (n.id, n.parents.clone())).collect();
        let generation = generations(&by_id);
        nodes.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then(generation[&b.id].cmp(&generation[&a.id]))
        });
        Ok(HistoryView {
            game: game.clone(),
            head: head_snap,
            branch: head.branch,
            detached: head.detached.is_some(),
            refs,
            nodes,
            locations: self.locations(&game.id)?,
        })
    }

    pub fn snapshot_files(&self, game: &GameRecord, spec: &str) -> Result<Vec<TreeEntry>> {
        let id = self.repo.resolve(&game.id, spec)?;
        Ok(self
            .repo
            .get_tree(&self.repo.get_snapshot(&id)?.tree)?
            .entries)
    }

    /// 跨游戏搜索备注。
    pub fn search_all_comments(&self, query: &str) -> Result<Vec<SearchHit>> {
        let names: HashMap<String, String> = self
            .index
            .games()?
            .into_iter()
            .map(|g| (g.id, g.name))
            .collect();
        Ok(self
            .index
            .search_comments(query, None)?
            .into_iter()
            .map(|r| SearchHit {
                game_name: names
                    .get(&r.game_id)
                    .cloned()
                    .unwrap_or_else(|| r.game_id.clone()),
                game_id: r.game_id,
                id: r.id,
                comment: r.comment,
                created_at: r.created_at,
            })
            .collect())
    }

    /// 最近一次可撤销的恢复（恢复后、下次备份前，界面常驻“撤销本次恢复”提示）。
    pub fn pending_undo(&self, game_id: &str) -> Result<Option<serde_json::Value>> {
        Ok(self
            .index
            .last_op(game_id)?
            .filter(|op| op.kind == "restore" && matches!(op.status.as_str(), "done" | "running"))
            .map(|op| op.detail))
    }
}

/// 每个快照的代数（没有父快照为 0，否则为父快照代数的最大值加 1）。用显式栈，避免长历史递归过深。
fn generations(parents: &HashMap<ObjectId, Vec<ObjectId>>) -> HashMap<ObjectId, u64> {
    let mut gen_of: HashMap<ObjectId, u64> = HashMap::new();
    for &start in parents.keys() {
        let mut stack = vec![start];
        while let Some(&id) = stack.last() {
            if gen_of.contains_key(&id) {
                stack.pop();
                continue;
            }
            let ps: Vec<ObjectId> = parents
                .get(&id)
                .map(|v| {
                    v.iter()
                        .filter(|p| parents.contains_key(p))
                        .copied()
                        .collect()
                })
                .unwrap_or_default();
            let pending: Vec<ObjectId> = ps
                .iter()
                .filter(|p| !gen_of.contains_key(p))
                .copied()
                .collect();
            if pending.is_empty() {
                let g = ps.iter().map(|p| gen_of[p] + 1).max().unwrap_or(0);
                gen_of.insert(id, g);
                stack.pop();
            } else {
                stack.extend(pending);
            }
        }
    }
    gen_of
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Env;

    #[test]
    fn generations_of_long_chain() {
        let ids: Vec<ObjectId> = (0..20_000u32)
            .map(|i| ObjectId::of(&i.to_le_bytes()))
            .collect();
        let mut parents = HashMap::new();
        parents.insert(ids[0], vec![]);
        for w in ids.windows(2) {
            parents.insert(w[1], vec![w[0]]);
        }
        let g = generations(&parents);
        assert_eq!(g[&ids[19_999]], 19_999);
    }
    use crate::{BackupOptions, RestoreOptions};

    #[test]
    fn rows_history_and_search() {
        let mut env = Env::new();
        env.write("a.sav", b"1");
        let g = env.add_game("空洞骑士");
        let lock = env.app.repo.lock().unwrap();
        env.app
            .backup(
                &lock,
                &g,
                &BackupOptions {
                    comment: Some("打完螳螂领主".into()),
                    tag: Some("螳螂".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        env.write("a.sav", b"2");
        env.app
            .backup(&lock, &g, &BackupOptions::default())
            .unwrap();
        drop(lock);

        let row = env.app.game_row(&g, false).unwrap();
        assert_eq!(
            (row.state, row.snapshots, row.files),
            (GameState::UpToDate, 2, 2)
        );
        assert!(row.last_backup_at.is_some());

        let h = env.app.history_view(&g).unwrap();
        assert_eq!(h.nodes.len(), 2);
        assert_eq!(h.nodes[1].refs, ["tags/螳螂"]);
        assert_eq!(h.nodes[0].parents, vec![h.nodes[1].id]);
        assert!(h.refs.iter().any(|r| r.current && r.name == "main"));
        assert_eq!(env.app.snapshot_files(&g, "HEAD").unwrap().len(), 2);

        let hits = env.app.search_all_comments("螳螂领主").unwrap();
        assert_eq!((hits.len(), hits[0].game_name.as_str()), (1, "空洞骑士"));

        assert!(env.app.pending_undo(&g.id).unwrap().is_none());
        let lock = env.app.repo.lock().unwrap();
        env.app
            .restore(&lock, &g, "HEAD~1", &RestoreOptions::default())
            .unwrap();
        drop(lock);
        assert!(env.app.pending_undo(&g.id).unwrap().is_some());
        env.write("a.sav", b"3");
        assert_eq!(env.app.changed_files(&g).unwrap().len(), 1);
    }
}

//! 总览页（0.3.0）：仓库空间统计、全部游戏的截图与磁盘对比、一键收集 / 写回。
//!
//! - 空间：仓库目录按“存档快照（对象库）/ 截图 / 其他”统计；每个游戏的存档快照占用按它的快照用到的对象计算
//!   （[`bonfire_core::usage`]），截图按它的截图目录计算；原始大小为全部快照中文件大小之和（不去重）。
//! - 增长曲线：每次统计时记下当天的仓库大小（缓存目录 `size-history.json`，保留 90 天）。
//! - 截图对比只读磁盘；写回只写入磁盘上没有的文件，不覆盖（与单个游戏的“与磁盘对比”相同）。

use std::collections::BTreeMap;
use std::path::Path;

use bonfire_core::index::GameRecord;
use bonfire_core::{ObjectId, RepoLock};
use serde::Serialize;

use crate::app::App;
use crate::error::Result;
use crate::gallery::WriteBackReport;

/// 增长曲线保留的天数。
const TREND_DAYS: usize = 90;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameUsage {
    pub id: String,
    pub name: String,
    pub snapshots: u64,
    /// 存档快照占用（磁盘上）。
    pub saves: u64,
    pub shots: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageStats {
    pub repo_dir: String,
    /// 仓库目录总大小。
    pub total: u64,
    /// 对象库（存档快照）。
    pub saves: u64,
    /// 截图（含回收站）。
    pub shots: u64,
    /// 索引、记录等其他文件。
    pub other: u64,
    /// 仓库所在磁盘的剩余空间。
    pub free: Option<u64>,
    /// 全部快照的原始大小（不去重、不压缩）。
    pub logical: u64,
    pub snapshots: u64,
    /// 按占用从大到小。
    pub games: Vec<GameUsage>,
    /// 每天的仓库大小（日期 → 字节），最早的在前。
    pub trend: Vec<(String, u64)>,
}

/// 一个游戏的截图与磁盘对比结果（汇总用）。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameDiskDiff {
    pub id: String,
    pub name: String,
    /// 磁盘上还没收集的新截图。
    pub new: usize,
    /// 与已收集的内容相同、不重复收集的。
    pub duplicates: usize,
    /// 图库中有、磁盘上没有的。
    pub missing: usize,
    /// 其中可以写回原位置的。
    pub restorable: usize,
    /// 其中原位置不在本机、会写到本机这个游戏截图文件夹的。
    pub to_home: usize,
    /// 其中本机没有可写位置的（需要选文件夹）。
    pub no_place: usize,
    /// 在回收站或已彻底删除、不再收集的。
    pub ignored: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskCompare {
    pub at: String,
    /// 只列出两边不一致的游戏。
    pub games: Vec<GameDiskDiff>,
    /// 参与对比的游戏数。
    pub compared: usize,
}

fn dir_size(p: &Path) -> u64 {
    walkdir::WalkDir::new(p)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// 路径所在磁盘的剩余空间。
fn free_space(p: &Path) -> Option<u64> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    // canonicalize 在 Windows 上带 `\\?\` 前缀（path_key 中为 `//?/`），去掉后才能与盘符比较。
    let key = crate::transfer::path_key(&p);
    let key = key.strip_prefix("//?/").unwrap_or(&key).to_string();
    disks
        .list()
        .iter()
        .filter(|d| key.starts_with(&crate::transfer::path_key(d.mount_point())))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

impl App {
    /// 统计仓库空间（可能需要几秒：要读每个快照的清单）。
    pub fn storage_stats(&self) -> Result<StorageStats> {
        let root = self.repo.root().to_path_buf();
        let total = dir_size(&root);
        let saves = dir_size(&self.repo.objects_dir());
        let shots = dir_size(&root.join(crate::shots::SHOTS_DIR));
        let mut st = StorageStats {
            repo_dir: root.to_string_lossy().into_owned(),
            total,
            saves,
            shots,
            other: total.saturating_sub(saves + shots),
            free: free_space(&root),
            ..Default::default()
        };
        for g in self.all_games()? {
            let snaps: Vec<ObjectId> = self
                .index
                .snapshots_of(&g.id)?
                .into_iter()
                .map(|s| s.id)
                .collect();
            let u = bonfire_core::usage::usage(&self.repo, &snaps)?;
            let shots = dir_size(&self.shot_dir(&g.id));
            st.logical += u.logical;
            st.snapshots += u.snapshots;
            if u.snapshots == 0 && shots == 0 {
                continue;
            }
            st.games.push(GameUsage {
                id: g.id,
                name: g.name,
                snapshots: u.snapshots,
                saves: u.stored,
                shots,
            });
        }
        st.games
            .sort_by_key(|g| std::cmp::Reverse(g.saves + g.shots));
        st.trend = self.record_size(total);
        Ok(st)
    }

    /// 记下今天的仓库大小，返回最近的记录。
    fn record_size(&self, total: u64) -> Vec<(String, u64)> {
        let path = self.paths.cache_dir.join("size-history.json");
        let mut m: BTreeMap<String, u64> = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let today = jiff::Zoned::now().date().to_string();
        m.insert(today, total);
        while m.len() > TREND_DAYS {
            let first = m.keys().next().cloned().expect("非空");
            m.remove(&first);
        }
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&path, serde_json::to_vec(&m).expect("可以序列化"));
        m.into_iter().collect()
    }

    /// 管理截图的游戏（游戏库中、管理方式含截图）。
    fn shot_games(&self) -> Result<Vec<GameRecord>> {
        Ok(self
            .games()?
            .into_iter()
            .filter(|g| self.shots_enabled(g).unwrap_or(false))
            .collect())
    }

    /// 全部游戏的截图与磁盘对比（只读）。
    pub fn compare_disk(&self) -> Result<DiskCompare> {
        let games = self.shot_games()?;
        let mut out = DiskCompare {
            at: crate::util::now_local_rfc3339(),
            compared: games.len(),
            games: Vec::new(),
        };
        for g in games {
            let mut d = GameDiskDiff {
                id: g.id.clone(),
                name: g.name.clone(),
                ..Default::default()
            };
            match self.shot_diff(&g) {
                Ok(diff) => {
                    d.new = diff.new.len();
                    d.duplicates = diff.duplicates;
                    d.missing = diff.missing.len();
                    d.restorable = diff.restorable;
                    d.ignored = diff.ignored.len();
                    let elsewhere = d.missing - d.restorable;
                    if diff.home.is_some() {
                        d.to_home = elsewhere;
                    } else {
                        d.no_place = elsewhere;
                    }
                }
                Err(e) => d.error = Some(e.to_string()),
            }
            if d.new + d.missing > 0 || d.error.is_some() {
                out.games.push(d);
            }
        }
        Ok(out)
    }

    /// 把全部游戏中“图库中有、磁盘上没有”的截图写到磁盘（原位置，或本机这个游戏的截图文件夹）。
    pub fn write_back_all(&mut self) -> Result<WriteBackReport> {
        let mut total = WriteBackReport::default();
        for g in self.shot_games()? {
            let blobs: Vec<ObjectId> = self.shot_diff(&g)?.missing.iter().map(|s| s.blob).collect();
            if blobs.is_empty() {
                continue;
            }
            let r = self.sync_shots_to_disk(&g, &blobs)?;
            total.written += r.written;
            total.bytes += r.bytes;
            total.exists += r.exists;
            total.no_dir += r.no_dir;
            for d in r.dirs {
                if !total.dirs.contains(&d) {
                    total.dirs.push(d);
                }
            }
        }
        Ok(total)
    }

    /// 收集全部游戏磁盘上的新截图，返回收集的张数。
    pub fn collect_all_shots(&mut self, lock: &RepoLock) -> Result<usize> {
        Ok(self.sync_all_shots(lock)?.iter().map(|r| r.added).sum())
    }
}

/// 稍后提醒的记录在界面状态中的键：值为 JSON `{ 条目键: 失败时间 }`；之后再失败（时间不同）时重新提醒。
pub const SNOOZE_KEY: &str = "overview.snoozed";

/// 需要处理的问题（红点、任务栏与托盘提示、系统通知；决策 OV-4）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionItem {
    /// `sync:<目标>`、`backup:<游戏>`、`fsck`。
    pub key: String,
    /// `sync`、`backup`、`fsck`。
    pub kind: String,
    /// 游戏名或目标名。
    pub name: String,
    pub game: Option<String>,
    pub target: Option<String>,
    pub issue: Option<String>,
    pub error: Option<String>,
    /// 失败的时间（也是稍后提醒的依据）。
    pub at: String,
    /// 同步的自动重试。
    pub retry: Option<crate::sync::Retry>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attention {
    pub items: Vec<AttentionItem>,
    /// 稍后提醒、暂不显示的条数。
    pub snoozed: usize,
}

impl App {
    /// 需要处理的问题：同步失败（没有忽略这类问题的）、最近一次备份失败、仓库校验发现问题。
    pub fn attention(&self) -> Result<Attention> {
        let mut items = Vec::new();
        for st in self.sync_status() {
            let t = &st.target;
            let Some(run) = st.last.filter(|r| r.error.is_some()) else {
                continue;
            };
            if !t.enabled {
                continue;
            }
            let issue = run.issue.unwrap_or(crate::sync::SyncIssue::Other);
            if t.ignores(issue) {
                continue;
            }
            items.push(AttentionItem {
                key: format!("sync:{}", t.id),
                kind: "sync".into(),
                name: t.name.clone(),
                target: Some(t.id.clone()),
                issue: Some(crate::activity::issue_key(issue).into()),
                error: run.error,
                at: run.at,
                retry: st.retry,
                ..Default::default()
            });
        }
        for a in self.open_failures()? {
            let kind = if a.action == "fsck" { "fsck" } else { "backup" };
            items.push(AttentionItem {
                key: match &a.game {
                    Some(g) if kind == "backup" => format!("backup:{g}"),
                    _ => kind.to_string(),
                },
                kind: kind.into(),
                name: a.name,
                game: a.game,
                issue: a.issue,
                error: a.error,
                at: a.at,
                ..Default::default()
            });
        }
        let state = crate::ui_state::load_ui_state(&crate::ui_state::ui_state_path(&self.paths));
        let snoozed: BTreeMap<String, String> = state
            .get(SNOOZE_KEY)
            .and_then(|v| serde_json::from_str(v).ok())
            .unwrap_or_default();
        let before = items.len();
        items.retain(|i| snoozed.get(&i.key) != Some(&i.at));
        Ok(Attention {
            snoozed: before - items.len(),
            items,
        })
    }

    /// 稍后提醒：直到这个问题再次发生（失败时间改变）。
    pub fn snooze(&self, key: &str, at: &str) -> Result<()> {
        let path = crate::ui_state::ui_state_path(&self.paths);
        let mut m: BTreeMap<String, String> = crate::ui_state::load_ui_state(&path)
            .get(SNOOZE_KEY)
            .and_then(|v| serde_json::from_str(v).ok())
            .unwrap_or_default();
        m.insert(key.to_string(), at.to_string());
        crate::ui_state::set_ui_state(
            &path,
            SNOOZE_KEY,
            Some(serde_json::to_string(&m).expect("可以序列化")),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::BackupOptions;
    use crate::testutil::Env;

    fn png(seed: u8) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(20, 10, image::Rgb([seed, 60, 90]))
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    /// 伪造根目录下的 Steam 截图文件夹（测试绝不接触真实目录）。
    fn shots_game(env: &mut Env) -> (GameRecord, std::path::PathBuf) {
        let root = env.app.fake_root.clone().unwrap();
        let dir = root.join("Steam/userdata/111/760/remote/504230/screenshots");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(root.join("Steam/steamapps")).unwrap();
        for i in 1..=3u8 {
            std::fs::write(dir.join(format!("2026101112000{i}_1.png")), png(i)).unwrap();
        }
        let g = GameRecord {
            id: "steam-504230".into(),
            name: "Celeste".into(),
            platform: "steam".into(),
            app_id: Some("504230".into()),
            install_dir: None,
            enabled: true,
            include_config: None,
            custom_rule_json: None,
        };
        env.app.index.upsert_game(&g).unwrap();
        (g, dir)
    }

    #[test]
    fn compare_collect_and_write_back_all() {
        let mut env = Env::new();
        let (_, dir) = shots_game(&mut env);
        let c = env.app.compare_disk().unwrap();
        assert_eq!(c.games.len(), 1);
        assert_eq!((c.games[0].new, c.games[0].missing), (3, 0));
        let lock = env.app.repo.lock().unwrap();
        assert_eq!(env.app.collect_all_shots(&lock).unwrap(), 3);
        drop(lock);
        assert!(env.app.compare_disk().unwrap().games.is_empty(), "两边一致");

        std::fs::remove_file(dir.join("20261011120002_1.png")).unwrap();
        let c = env.app.compare_disk().unwrap();
        assert_eq!(
            (
                c.games[0].missing,
                c.games[0].restorable,
                c.games[0].no_place
            ),
            (1, 1, 0)
        );
        let r = env.app.write_back_all().unwrap();
        assert_eq!(r.written, 1);
        assert_eq!(
            std::fs::read(dir.join("20261011120002_1.png")).unwrap(),
            png(2)
        );
        assert!(env.app.compare_disk().unwrap().games.is_empty());

        let log = env.app.activities(&Default::default()).unwrap();
        let actions: Vec<&str> = log.items.iter().map(|a| a.action.as_str()).collect();
        assert_eq!(actions, ["shots-write-back", "shots-collect"]);
    }

    #[test]
    fn storage_counts_saves_shots_and_trend() {
        let mut env = Env::new();
        let (g, _) = shots_game(&mut env);
        env.write("a.sav", &vec![5u8; 50_000]);
        let save = env.add_game("Hades");
        let lock = env.app.repo.lock().unwrap();
        env.app
            .backup(&lock, &save, &BackupOptions::default())
            .unwrap();
        env.write("a.sav", &vec![6u8; 50_000]);
        env.app
            .backup(&lock, &save, &BackupOptions::default())
            .unwrap();
        env.app.sync_shots(&lock, &g).unwrap();
        drop(lock);

        let st = env.app.storage_stats().unwrap();
        assert!(st.total >= st.saves + st.shots && st.saves > 0 && st.shots > 0);
        assert_eq!(st.snapshots, 2);
        assert!(
            (100_000..100_100).contains(&st.logical),
            "两次快照的原始大小之和"
        );
        assert!(st.saves < st.logical, "压缩后小于原始大小");
        let hades = st.games.iter().find(|x| x.name == "Hades").unwrap();
        assert_eq!((hades.snapshots, hades.shots), (2, 0));
        assert!(hades.saves > 0);
        let celeste = st.games.iter().find(|x| x.name == "Celeste").unwrap();
        assert!(celeste.shots > 0 && celeste.saves == 0);
        assert_eq!(st.trend.len(), 1);
        assert_eq!(st.trend[0].1, st.total);
        assert!(st.free.is_some(), "临时目录所在磁盘的剩余空间");
    }

    #[test]
    fn attention_lists_failures_and_snoozes() {
        let mut env = Env::new();
        env.write("a.sav", b"one");
        let g = env.add_game("Hades");
        assert!(env.app.attention().unwrap().items.is_empty());
        let mut fail = crate::activity::Activity::new("backup-auto");
        fail.game = Some(g.id.clone());
        fail.name = g.name.clone();
        fail.ok = false;
        fail.issue = Some("locked".into());
        env.app.note(fail.clone());
        let a = env.app.attention().unwrap();
        assert_eq!(a.items.len(), 1);
        assert_eq!(
            (a.items[0].key.as_str(), a.items[0].issue.as_deref()),
            (format!("backup:{}", g.id).as_str(), Some("locked"))
        );
        env.app.snooze(&a.items[0].key, &a.items[0].at).unwrap();
        let a = env.app.attention().unwrap();
        assert_eq!((a.items.len(), a.snoozed), (0, 1));
        // 之后成功备份：问题解决。
        let lock = env.app.repo.lock().unwrap();
        env.app
            .backup(&lock, &g, &BackupOptions::default())
            .unwrap();
        drop(lock);
        assert_eq!(env.app.attention().unwrap().snoozed, 0);

        // 同步失败：忽略“连不上目标”后不再算作需要处理。
        let blocker = env.dir.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let t = crate::sync::SyncTarget::new(
            crate::sync::SyncKind::Folder,
            "硬盘",
            &blocker.join("sub").to_string_lossy(),
        );
        let mut t = env.app.save_sync_target(t, None).unwrap();
        assert!(env.app.sync_push(&t.id, &mut |_, _| {}).is_err());
        let a = env.app.attention().unwrap();
        assert_eq!((a.items.len(), a.items[0].kind.as_str()), (1, "sync"));
        t.ignore = vec![crate::sync::SyncIssue::Network];
        env.app.save_sync_target(t, None).unwrap();
        assert!(env.app.attention().unwrap().items.is_empty());
    }
}

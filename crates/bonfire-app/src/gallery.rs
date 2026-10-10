//! 图库（U-10）：所有游戏的截图汇总；截图备份与磁盘上截图文件夹的对比与双向同步。
//!
//! - 对比：磁盘上有、还没收集的（新截图）；磁盘上有、但在回收站中或已彻底删除的（不会再收集）；
//!   备份中有、磁盘上原位置已经没有的（可以写回）。
//! - 写回：把备份中的原图复制回原位置或指定的文件夹。只写入不存在的文件，绝不覆盖；
//!   原文件夹已不存在时跳过（不替用户创建游戏的目录）。
//! - 重新收集：回收站中的恢复到相册；彻底删除的去掉删除记录后重新收集。

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use bonfire_core::ObjectId;
use bonfire_core::RepoLock;
use bonfire_core::index::{GameRecord, ShotRecord};
use bonfire_discovery::shots::ShotFile;
use serde::Serialize;

use crate::app::App;
use crate::error::{AppError, Result};
use crate::shots::{ManageMode, SHOTS_DIR, ShotSyncReport};
use crate::transfer::with_suffix;

/// 图库中的一个游戏。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryGame {
    pub id: String,
    pub name: String,
    pub name_zh: Option<String>,
    pub platform: String,
    /// 已移出游戏库的游戏为 false（截图仍保留）。
    pub in_library: bool,
    pub mode: ManageMode,
}

/// 图库：所有游戏的截图（相册与回收站）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryView {
    pub games: Vec<GalleryGame>,
    /// 相册中的截图，按拍摄时间从新到旧。
    pub shots: Vec<ShotRecord>,
    pub trash: Vec<ShotRecord>,
    pub bytes: u64,
    pub trash_bytes: u64,
    /// 所有游戏的截图原图所在的文件夹。
    pub dir: PathBuf,
}

/// 磁盘上的一张截图。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskShot {
    pub path: PathBuf,
    pub size: u64,
    pub source: String,
}

/// 磁盘上还在、但在回收站中或已彻底删除的截图。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IgnoredShot {
    pub path: PathBuf,
    pub size: u64,
    pub blob: ObjectId,
    /// true 为已彻底删除，false 为在回收站中。
    pub purged: bool,
}

/// 截图备份与磁盘上截图文件夹的对比。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotDiff {
    pub game_id: String,
    /// 磁盘上有、备份中没有：还没收集的新截图。
    pub new: Vec<DiskShot>,
    /// 磁盘上有、内容与相册中已有的截图相同（换了文件名或位置），不会重复收集。
    pub duplicates: usize,
    /// 磁盘上有、但在回收站中或已彻底删除，不会再收集。
    pub ignored: Vec<IgnoredShot>,
    /// 备份中有、磁盘上原位置已经没有。
    pub missing: Vec<ShotRecord>,
    /// `missing` 中原文件夹还在、可以写回原位置的数量。
    pub restorable: usize,
    /// 两边都有的截图数。
    pub synced: usize,
}

/// 写回磁盘的结果。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteBackReport {
    pub written: usize,
    pub bytes: u64,
    /// 目标已有同名文件，未覆盖。
    pub exists: usize,
    /// 原文件夹已不存在，未写入。
    pub no_dir: usize,
    /// 写入的文件夹（去重）。
    pub dirs: Vec<PathBuf>,
}

impl App {
    /// 图库：所有游戏（含已移出游戏库、仍有截图的游戏）的截图。
    pub fn gallery(&self) -> Result<GalleryView> {
        let mut games = Vec::new();
        let mut shots = Vec::new();
        let mut trash = Vec::new();
        for g in self.index.games()? {
            let list: Vec<ShotRecord> = self
                .index
                .shots_of(&g.id)?
                .into_iter()
                .filter(|s| !s.purged)
                .collect();
            if list.is_empty() && !g.enabled {
                continue;
            }
            let (t, a): (Vec<ShotRecord>, Vec<ShotRecord>) =
                list.into_iter().partition(|s| s.deleted_at.is_some());
            shots.extend(a);
            trash.extend(t);
            games.push(GalleryGame {
                name_zh: self.metadata(&g.id)?.and_then(|m| m.data.name_zh),
                mode: self.manage_mode(&g)?,
                in_library: g.enabled,
                id: g.id,
                name: g.name,
                platform: g.platform,
            });
        }
        let newest = |a: &ShotRecord, b: &ShotRecord| b.taken_at.cmp(&a.taken_at);
        shots.sort_by(newest);
        trash.sort_by(|a, b| b.deleted_at.cmp(&a.deleted_at).then(newest(a, b)));
        Ok(GalleryView {
            bytes: shots.iter().map(|s| s.size).sum(),
            trash_bytes: trash.iter().map(|s| s.size).sum(),
            games,
            shots,
            trash,
            dir: self.repo.root().join(SHOTS_DIR),
        })
    }

    /// 对比截图备份与磁盘上的截图文件夹（只读）。
    pub fn shot_diff(&self, game: &GameRecord) -> Result<ShotDiff> {
        let records = self.index.shots_of(&game.id)?;
        let on_disk = self.shot_files(game)?;
        // 路径按规范化后比较（大小写、斜杠方向不同的同一文件算同一个）。
        let key = |p: &Path| crate::transfer::path_key(p);
        let disk_keys: HashSet<(String, u64)> =
            on_disk.iter().map(|f| (key(&f.path), f.size)).collect();
        let by_key: HashMap<(String, u64), &ShotRecord> = records
            .iter()
            .map(|s| ((key(Path::new(&s.original_path)), s.size), s))
            .collect();
        let active: HashSet<ObjectId> = records
            .iter()
            .filter(|s| !s.purged && s.deleted_at.is_none())
            .map(|s| s.blob)
            .collect();

        let mut diff = ShotDiff {
            game_id: game.id.clone(),
            new: Vec::new(),
            duplicates: 0,
            ignored: Vec::new(),
            missing: Vec::new(),
            restorable: 0,
            synced: 0,
        };
        for f in &on_disk {
            match by_key.get(&(key(&f.path), f.size)) {
                Some(s) if s.purged || s.deleted_at.is_some() => diff.ignored.push(IgnoredShot {
                    path: f.path.clone(),
                    size: f.size,
                    blob: s.blob,
                    purged: s.purged,
                }),
                Some(_) => diff.synced += 1,
                None => {
                    // 新文件：内容与相册中已有的相同时只是换了名字或位置。
                    let dup = std::fs::read(&f.path)
                        .map(|d| active.contains(&ObjectId::of(&d)))
                        .unwrap_or(false);
                    if dup {
                        diff.duplicates += 1;
                    } else {
                        diff.new.push(DiskShot {
                            path: f.path.clone(),
                            size: f.size,
                            source: f.source.as_str().to_string(),
                        });
                    }
                }
            }
        }
        for s in records {
            if s.purged || s.deleted_at.is_some() {
                continue;
            }
            if disk_keys.contains(&(key(Path::new(&s.original_path)), s.size)) {
                continue;
            }
            let p = Path::new(&s.original_path);
            if p.is_file() {
                // 原位置有同名文件但内容变了：不算缺失，也不覆盖。
                continue;
            }
            if p.parent().is_some_and(Path::is_dir) {
                diff.restorable += 1;
            }
            diff.missing.push(s);
        }
        diff.new.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(diff)
    }

    /// 把备份中有、磁盘上没有的截图写回（`blobs` 为空表示全部缺失的）。
    /// `dest` 为空时写回原位置（原文件夹不存在则跳过），否则写入 `dest` 文件夹。只写入不存在的文件，不覆盖。
    pub fn write_back_shots(
        &self,
        game: &GameRecord,
        blobs: &[ObjectId],
        dest: Option<&Path>,
    ) -> Result<WriteBackReport> {
        if let Some(d) = dest {
            if !d.is_dir() {
                return Err(AppError::PathNotFound(d.to_path_buf()));
            }
            self.check_export_target(d)?;
        }
        let wanted: HashSet<&ObjectId> = blobs.iter().collect();
        let mut report = WriteBackReport::default();
        for s in self.shot_diff(game)?.missing {
            if !wanted.is_empty() && !wanted.contains(&s.blob) {
                continue;
            }
            let original = PathBuf::from(&s.original_path);
            let target = match dest {
                Some(d) => d.join(
                    original
                        .file_name()
                        .unwrap_or_else(|| std::ffi::OsStr::new(&s.file)),
                ),
                None => original,
            };
            let Some(dir) = target.parent().filter(|d| d.is_dir()) else {
                report.no_dir += 1;
                continue;
            };
            if target.exists() {
                report.exists += 1;
                continue;
            }
            let data = self.shot_bytes(&s)?;
            write_new(&target, &data)?;
            report.written += 1;
            report.bytes += data.len() as u64;
            if !report.dirs.iter().any(|d| d == dir) {
                report.dirs.push(dir.to_path_buf());
            }
        }
        if report.written > 0 {
            tracing::info!(game = %game.id, n = report.written, "已把截图写回磁盘");
        }
        Ok(report)
    }

    /// 本机这个游戏的截图文件夹（U-16）：磁盘上已有这个游戏截图的文件夹（最近一张所在的），
    /// 其次为游戏添加的截图文件夹，再其次 Steam 中这个游戏的截图文件夹。都没有时为 None。
    pub fn shot_home(&self, game: &GameRecord) -> Result<Option<PathBuf>> {
        let files = self.shot_files(game)?;
        if let Some(f) = files.iter().max_by_key(|f| f.mtime_ns)
            && let Some(dir) = f.path.parent()
        {
            return Ok(Some(dir.to_path_buf()));
        }
        if let Some(d) = self
            .index
            .shot_settings(&game.id)?
            .dirs
            .into_iter()
            .map(PathBuf::from)
            .find(|d| d.is_dir())
        {
            return Ok(Some(d));
        }
        // Steam：`userdata/<用户>/760/remote/<AppID>/screenshots`（文件夹已存在时；不替 Steam 创建目录）。
        if game.platform == "steam"
            && let (Some(app_id), Some(root)) = (&game.app_id, self.steam_root())
            && let Ok(users) = std::fs::read_dir(root.join("userdata"))
        {
            for u in users.flatten() {
                let d = u
                    .path()
                    .join("760")
                    .join("remote")
                    .join(app_id)
                    .join("screenshots");
                if d.is_dir() {
                    return Ok(Some(d));
                }
            }
        }
        Ok(None)
    }

    /// 把相册中的截图（如刚导入的）同步到磁盘（U-16）：原位置的文件夹在本机存在时写回原位置，
    /// 否则写到本机这个游戏的截图文件夹（[`App::shot_home`]）；写到新位置时记录的原位置改为新位置，
    /// 之后不会被当作新截图。只写入不存在的文件，不覆盖。
    pub fn sync_shots_to_disk(
        &mut self,
        game: &GameRecord,
        blobs: &[ObjectId],
    ) -> Result<WriteBackReport> {
        let wanted: HashSet<&ObjectId> = blobs.iter().collect();
        let home = self.shot_home(game)?;
        let mut report = WriteBackReport::default();
        let mut moved = false;
        for mut s in self.index.shots_of(&game.id)? {
            if s.purged || s.deleted_at.is_some() || !wanted.contains(&s.blob) {
                continue;
            }
            let original = PathBuf::from(&s.original_path);
            if original.is_file() {
                continue;
            }
            let target = if original.parent().is_some_and(Path::is_dir) {
                original.clone()
            } else if let Some(h) = &home {
                h.join(
                    original
                        .file_name()
                        .unwrap_or_else(|| std::ffi::OsStr::new(&s.file)),
                )
            } else {
                report.no_dir += 1;
                continue;
            };
            if target.exists() {
                report.exists += 1;
                continue;
            }
            let data = self.shot_bytes(&s)?;
            write_new(&target, &data)?;
            report.written += 1;
            report.bytes += data.len() as u64;
            if let Some(dir) = target.parent()
                && !report.dirs.iter().any(|d| d == dir)
            {
                report.dirs.push(dir.to_path_buf());
            }
            if target != original {
                s.original_path = target.to_string_lossy().into_owned();
                self.index.update_shot(&s)?;
                moved = true;
            }
        }
        if moved {
            self.save_album(&game.id)?;
        }
        Ok(report)
    }

    /// 重新收集磁盘上被忽略的截图（`paths` 为空表示全部）：回收站中的恢复到相册，
    /// 彻底删除的去掉删除记录后重新收集。返回回到相册的数量。
    pub fn recollect_shots(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        paths: &[PathBuf],
    ) -> Result<usize> {
        let wanted: HashSet<&PathBuf> = paths.iter().collect();
        let ignored: Vec<IgnoredShot> = self
            .shot_diff(game)?
            .ignored
            .into_iter()
            .filter(|i| wanted.is_empty() || wanted.contains(&i.path))
            .collect();
        let trashed: Vec<ObjectId> = ignored
            .iter()
            .filter(|i| !i.purged)
            .map(|i| i.blob)
            .collect();
        let mut n = self.restore_shots(lock, game, &trashed)?;
        let purged: Vec<&IgnoredShot> = ignored.iter().filter(|i| i.purged).collect();
        if !purged.is_empty() {
            let blobs: Vec<ObjectId> = purged.iter().map(|i| i.blob).collect();
            self.index.delete_shots(&game.id, &blobs)?;
            let keys: HashSet<&Path> = purged.iter().map(|i| i.path.as_path()).collect();
            let files: Vec<ShotFile> = self
                .shot_files(game)?
                .into_iter()
                .filter(|f| keys.contains(f.path.as_path()))
                .collect();
            let report = ShotSyncReport {
                game_id: game.id.clone(),
                ..Default::default()
            };
            n += self.collect_files(game, files, report)?.added;
            self.save_album(&game.id)?;
        }
        Ok(n)
    }
}

/// 写入新文件（先写临时文件再改名；目标已存在时报错，不覆盖）。
fn write_new(target: &Path, data: &[u8]) -> Result<()> {
    let tmp = with_suffix(target, ".bonfire-tmp");
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()
    };
    write().map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        AppError::io(&tmp, e)
    })?;
    if target.exists() {
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::Invalid(format!(
            "目标已存在，未覆盖：{}",
            target.display()
        )));
    }
    std::fs::rename(&tmp, target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        AppError::io(target, e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Env;

    fn png(seed: u8) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(20, 10, image::Rgb([seed, 30, 90]))
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    /// 伪造根目录下的 Steam 截图文件夹（测试绝不接触真实目录）。
    fn setup(env: &mut Env) -> (GameRecord, PathBuf) {
        let root = env.app.fake_root.clone().unwrap();
        let dir = root.join("Steam/userdata/111/760/remote/504230/screenshots");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(root.join("Steam/steamapps")).unwrap();
        for i in 1..=4u8 {
            std::fs::write(dir.join(format!("2026100912000{i}_1.png")), png(i)).unwrap();
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
    fn manage_mode_switches() {
        let mut env = Env::new();
        let (g, _) = setup(&mut env);
        assert_eq!(env.app.manage_mode(&g).unwrap(), ManageMode::Both);
        env.app.set_manage_mode(&g, ManageMode::Saves).unwrap();
        assert_eq!(env.app.manage_mode(&g).unwrap(), ManageMode::Saves);
        assert!(!env.app.shots_enabled(&g).unwrap());
        env.app.set_manage_mode(&g, ManageMode::Shots).unwrap();
        assert_eq!(env.app.manage_mode(&g).unwrap(), ManageMode::Shots);
        assert!(env.app.shots_enabled(&g).unwrap());
        // 全局关闭截图时，只管理截图的游戏仍然收集。
        env.app.config.shots.enabled = false;
        env.app.set_manage_mode(&g, ManageMode::Shots).unwrap();
        assert!(env.app.shots_enabled(&g).unwrap());
        env.app.set_manage_mode(&g, ManageMode::Both).unwrap();
        assert_eq!(env.app.manage_mode(&g).unwrap(), ManageMode::Both);
        assert_eq!("saves".parse::<ManageMode>().unwrap(), ManageMode::Saves);
        assert!("x".parse::<ManageMode>().is_err());
    }

    #[test]
    fn diff_write_back_and_recollect() {
        let mut env = Env::new();
        let (g, dir) = setup(&mut env);
        let lock = env.app.repo.lock().unwrap();
        let d = env.app.shot_diff(&g).unwrap();
        assert_eq!((d.new.len(), d.synced), (4, 0));
        env.app.sync_shots(&lock, &g).unwrap();

        // 磁盘上删掉 1 张；回收站 1 张；彻底删除 1 张；换名复制 1 张。
        let shots = env.app.album(&g).unwrap().shots;
        let by_name = |n: &str| {
            shots
                .iter()
                .find(|s| s.original_path.ends_with(n))
                .unwrap()
                .blob
        };
        std::fs::remove_file(dir.join("20261009120001_1.png")).unwrap();
        env.app
            .delete_shots(&lock, &g, &[by_name("20261009120002_1.png")])
            .unwrap();
        env.app
            .purge_shots(&lock, &g, &[by_name("20261009120003_1.png")])
            .unwrap();
        std::fs::write(dir.join("copy.png"), png(4)).unwrap();
        std::fs::write(dir.join("20261009120009_1.png"), png(9)).unwrap();

        let d = env.app.shot_diff(&g).unwrap();
        assert_eq!(d.new.len(), 1, "只有真正的新截图");
        assert_eq!(d.duplicates, 1);
        assert_eq!(d.ignored.len(), 2);
        assert_eq!(d.ignored.iter().filter(|i| i.purged).count(), 1);
        assert_eq!(d.missing.len(), 1);
        assert_eq!(d.restorable, 1);
        assert_eq!(d.synced, 1);

        // 写回原位置：内容与原图一致；再写一次不覆盖（已经不缺了）。
        let r = env.app.write_back_shots(&g, &[], None).unwrap();
        assert_eq!(r.written, 1);
        assert_eq!(
            std::fs::read(dir.join("20261009120001_1.png")).unwrap(),
            png(1)
        );
        assert_eq!(env.app.shot_diff(&g).unwrap().missing.len(), 0);
        assert_eq!(env.app.write_back_shots(&g, &[], None).unwrap().written, 0);

        // 写到指定文件夹；目标已有同名文件时不覆盖。
        std::fs::remove_file(dir.join("20261009120001_1.png")).unwrap();
        let out = env.dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("20261009120001_1.png"), b"keep").unwrap();
        let r = env.app.write_back_shots(&g, &[], Some(&out)).unwrap();
        assert_eq!((r.written, r.exists), (0, 1));
        assert_eq!(
            std::fs::read(out.join("20261009120001_1.png")).unwrap(),
            b"keep"
        );
        // 不能写入仓库。
        let repo = env.app.repo.root().to_path_buf();
        assert!(env.app.write_back_shots(&g, &[], Some(&repo)).is_err());

        // 重新收集被忽略的：回收站中的恢复，彻底删除的重新收集。
        let n = env.app.recollect_shots(&lock, &g, &[]).unwrap();
        assert_eq!(n, 2);
        let d = env.app.shot_diff(&g).unwrap();
        assert!(d.ignored.is_empty());
        assert_eq!(env.app.album(&g).unwrap().shots.len(), 4);
        assert_eq!(env.app.album(&g).unwrap().trash.len(), 0);
    }

    #[test]
    fn package_with_saves_and_shots_round_trip() {
        use crate::transfer::{ArchivePick, PackageOptions};
        let mut env = Env::new();
        let (g, dir1) = setup(&mut env);
        let lock = env.app.repo.lock().unwrap();
        env.app.sync_shots(&lock, &g).unwrap();
        let picked = env.app.album(&g).unwrap().shots[0].blob;
        // 只导出选中的一张截图（不含存档：这个游戏还没有快照）。
        let out = env.dir.path().join("p.bonfire");
        let r = env
            .app
            .export_package(
                std::slice::from_ref(&g),
                &out,
                &PackageOptions {
                    saves: true,
                    shots: true,
                    shot_ids: [(g.id.clone(), vec![picked])].into(),
                },
            )
            .unwrap();
        assert_eq!(
            (r.games.len(), r.games[0].snapshots, r.games[0].shots),
            (1, 0, 1)
        );
        // 什么都不选时拒绝。
        assert!(
            env.app
                .export_package(std::slice::from_ref(&g), &out, &PackageOptions::default())
                .is_err()
        );

        // 在另一个仓库导入：不选截图时什么都不导入；选截图时合并到相册，并写到磁盘。
        // 另一台电脑：原来的伪造目录已不存在。
        drop(lock);
        let mut other = Env::new();
        let moved = other.dir.path().join("p.bonfire");
        std::fs::copy(&out, &moved).unwrap();
        let out = moved;
        drop(env);
        let _ = std::fs::remove_dir_all(&dir1);
        let (g2, dir2) = setup(&mut other);
        for f in std::fs::read_dir(&dir2).unwrap() {
            std::fs::remove_file(f.unwrap().path()).unwrap();
        }
        let lock2 = other.app.repo.lock().unwrap();
        let preview = other.app.inspect_archive(&out).unwrap();
        let key = preview.games[0].key.clone();
        let mut pick = ArchivePick::all(key.clone());
        pick.shots = false;
        let items = other
            .app
            .import_archive(&lock2, &out, &[pick], false)
            .unwrap();
        assert_eq!(items[0].report.as_ref().unwrap().imported_shots, 0);
        let items = other
            .app
            .import_archive(&lock2, &out, &[ArchivePick::all(key)], false)
            .unwrap();
        let rep = items[0].report.as_ref().unwrap();
        assert_eq!(rep.imported_shots, 1);
        assert_eq!(rep.new_shot_ids, vec![picked]);
        // 包内记录的原位置在另一个伪造根目录下，本机不存在：写到本机这个游戏的 Steam 截图文件夹。
        let w = other
            .app
            .sync_shots_to_disk(&g2, &rep.new_shot_ids)
            .unwrap();
        assert_eq!(w.written, 1, "{w:?}");
        assert_eq!(std::fs::read_dir(&dir2).unwrap().count(), 1);
        // 记录的原位置改为新位置：之后不会被当作新截图。
        assert_eq!(other.app.pending_shots(&g2).unwrap(), 0);
        assert_eq!(
            other
                .app
                .sync_shots_to_disk(&g2, &rep.new_shot_ids)
                .unwrap()
                .written,
            0
        );
    }

    #[test]
    fn gallery_lists_all_games() {
        let mut env = Env::new();
        let (g, _) = setup(&mut env);
        let lock = env.app.repo.lock().unwrap();
        env.app.sync_shots(&lock, &g).unwrap();
        let first = env.app.album(&g).unwrap().shots[0].blob;
        env.app.delete_shots(&lock, &g, &[first]).unwrap();
        let v = env.app.gallery().unwrap();
        assert_eq!((v.shots.len(), v.trash.len()), (3, 1));
        assert!(v.shots.windows(2).all(|w| w[0].taken_at >= w[1].taken_at));
        let game = v.games.iter().find(|x| x.id == g.id).unwrap();
        assert!(game.in_library);
        assert_eq!(game.mode, ManageMode::Both);
        assert!(v.dir.ends_with(SHOTS_DIR));
    }
}

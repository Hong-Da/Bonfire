//! 未识别游戏的截图（U-29）：“捕获”文件夹与公用截图文件夹中对应不上游戏库中任何游戏的截图，
//! 也照常备份，放在单独的相册“未识别的截图”（游戏 ID [`UNSORTED`]，不是游戏库中的游戏）。
//!
//! - 收集：随定时 / 变化后收集、`shots sync`（不带游戏）一起进行；设置 `shots.keep_unsorted` 关闭时不收集。
//! - 自动归位：之后游戏库中加入了对应的游戏，这个游戏收集截图时，相册中原路径属于它的截图会移过去，
//!   不会重复收集。
//! - 手动归类：[`App::assign_unsorted`] 把选中的截图归到某个游戏。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use bonfire_core::index::{GameRecord, ShotRecord};
use bonfire_core::{ObjectId, RepoLock};
use bonfire_discovery::shots::{self as src, ShotFile};

use crate::app::App;
use crate::error::{AppError, Result};
use crate::shots::ShotSyncReport;

/// “未识别的截图”相册的游戏 ID。
pub const UNSORTED: &str = "unsorted";

/// “未识别的截图”相册对应的游戏记录（只在内存中，不加入游戏库）。
pub fn unsorted_game() -> GameRecord {
    GameRecord {
        id: UNSORTED.into(),
        name: "未识别的截图".into(),
        platform: UNSORTED.into(),
        app_id: None,
        install_dir: None,
        enabled: true,
        include_config: None,
        custom_rule_json: None,
    }
}

fn key(p: &Path) -> String {
    crate::transfer::path_key(p)
}

impl App {
    /// 游戏库中所有游戏的名称（对应截图用）。
    fn library_shot_names(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        for g in self.games()? {
            names.extend(self.shot_names(&g)?);
        }
        Ok(names)
    }

    /// 磁盘上对应不上游戏库中任何游戏的截图（“捕获”文件夹与公用截图文件夹）。
    pub fn unsorted_files(&self) -> Result<Vec<ShotFile>> {
        let names = self.library_shot_names()?;
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut out = Vec::new();
        if let Some(cap) = self.captures_dir().filter(|d| d.is_dir()) {
            out.extend(src::capture_unmatched(&cap, &refs));
        }
        for d in self.shared_dirs() {
            out.extend(src::shared_unmatched(&d, &refs));
        }
        Ok(out)
    }

    /// 收集未识别游戏的截图到“未识别的截图”相册。
    pub fn sync_unsorted(&mut self, _lock: &RepoLock) -> Result<ShotSyncReport> {
        let game = unsorted_game();
        let report = ShotSyncReport {
            game_id: game.id.clone(),
            ..Default::default()
        };
        if !self.config.shots.enabled || !self.config.shots.keep_unsorted {
            return Ok(ShotSyncReport {
                disabled: true,
                ..report
            });
        }
        let known: HashSet<(String, u64)> = self
            .index
            .shots_of(UNSORTED)?
            .into_iter()
            .map(|s| (key(Path::new(&s.original_path)), s.size))
            .collect();
        let files: Vec<ShotFile> = self
            .unsorted_files()?
            .into_iter()
            .filter(|f| !known.contains(&(key(&f.path), f.size)))
            .collect();
        self.collect_files(&game, files, report)
    }

    /// 自动归位：“未识别的截图”中原路径属于这个游戏的截图移到这个游戏的相册。返回移动的张数。
    pub(crate) fn adopt_unsorted(&mut self, game: &GameRecord) -> Result<usize> {
        if game.id == UNSORTED || self.index.shots_of(UNSORTED)?.is_empty() {
            return Ok(0);
        }
        let mine: HashSet<String> = self
            .shot_files(game)?
            .iter()
            .map(|f| key(&f.path))
            .collect();
        let blobs: Vec<ObjectId> = self
            .index
            .shots_of(UNSORTED)?
            .into_iter()
            .filter(|s| mine.contains(&key(Path::new(&s.original_path))))
            .map(|s| s.blob)
            .collect();
        if blobs.is_empty() {
            return Ok(0);
        }
        let n = self.move_unsorted(&blobs, game)?;
        tracing::info!(game = %game.id, n, "未识别的截图已归到游戏");
        Ok(n)
    }

    /// 手动归类：把“未识别的截图”中的这些截图归到 `game`（回收站中的保持在回收站）。
    pub fn assign_unsorted(
        &mut self,
        _lock: &RepoLock,
        blobs: &[ObjectId],
        game: &GameRecord,
    ) -> Result<usize> {
        if game.id == UNSORTED {
            return Err(AppError::Invalid("请选择游戏库中的游戏".into()));
        }
        self.move_unsorted(blobs, game)
    }

    /// 把截图记录与原图从“未识别的截图”移到 `game`。目标游戏已有相同内容的截图时只去掉未识别中的这一张。
    fn move_unsorted(&mut self, blobs: &[ObjectId], game: &GameRecord) -> Result<usize> {
        let wanted: HashSet<&ObjectId> = blobs.iter().collect();
        let (moving, keep): (Vec<ShotRecord>, Vec<ShotRecord>) = self
            .index
            .shots_of(UNSORTED)?
            .into_iter()
            .partition(|s| wanted.contains(&s.blob));
        if moving.is_empty() {
            return Ok(0);
        }
        let mut target: Vec<ShotRecord> = self.index.shots_of(&game.id)?;
        let have: HashMap<ObjectId, usize> = target
            .iter()
            .enumerate()
            .map(|(i, s)| (s.blob, i))
            .collect();
        let from_dir = self.shot_dir(UNSORTED);
        let to_dir = self.shot_dir(&game.id);
        let mut moved = 0;
        for mut s in moving {
            if have.contains_key(&s.blob) {
                // 目标中已有：删除未识别中的原图即可。
                if !s.file.is_empty() {
                    let _ = std::fs::remove_file(from_dir.join(&s.file));
                }
                moved += 1;
                continue;
            }
            if !s.file.is_empty() {
                let from = from_dir.join(&s.file);
                let to = free_path(&to_dir.join(&s.file));
                if let Some(dir) = to.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
                }
                std::fs::rename(&from, &to).map_err(|e| AppError::io(&from, e))?;
                s.file = to
                    .strip_prefix(&to_dir)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or(s.file);
            }
            s.game_id = game.id.clone();
            target.push(s);
            moved += 1;
        }
        self.index.replace_shots(&game.id, &target)?;
        self.index.replace_shots(UNSORTED, &keep)?;
        self.save_album(&game.id)?;
        self.save_album(UNSORTED)?;
        Ok(moved)
    }
}

/// 不覆盖已有文件：重名时加序号。
fn free_path(p: &Path) -> PathBuf {
    if !p.exists() {
        return p.to_path_buf();
    }
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = p
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    let dir = p.parent().map(Path::to_path_buf).unwrap_or_default();
    (2..)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|c| !c.exists())
        .expect("总能找到不重名的文件名")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Env;

    fn png(seed: u8) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(20, 10, image::Rgb([seed, 40, 90]))
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    fn put(p: &Path, data: &[u8]) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }

    #[test]
    fn unmatched_shots_are_kept_then_adopted_or_assigned() {
        let mut env = Env::new();
        let cap = env
            .app
            .known_dirs
            .home
            .clone()
            .unwrap()
            .join("Videos/Captures");
        put(&cap.join("Minishoot 2026_7_18 0_02_36.png"), &png(1));
        put(&cap.join("Hollow Knight 2026_10_9 21_05_00.png"), &png(2));
        let shared = env.app.fake_root.clone().unwrap().join("Shots");
        put(&shared.join("Celeste/a.png"), &png(3));
        put(&shared.join("IMG_0001.png"), &png(4));
        env.app.config.shots.shared_dirs = vec![shared.clone()];
        let hk = env.add_game("Hollow Knight");
        let lock = env.app.repo.lock().unwrap();

        // 对应不上游戏的 3 张进入“未识别的截图”；Hollow Knight 的不进。
        let r = env.app.sync_unsorted(&lock).unwrap();
        assert_eq!(r.added, 3);
        assert_eq!(env.app.sync_unsorted(&lock).unwrap().added, 0);
        let names: Vec<String> = env
            .app
            .index
            .shots_of(UNSORTED)
            .unwrap()
            .iter()
            .map(|s| {
                Path::new(&s.original_path)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert!(names.contains(&"Minishoot 2026_7_18 0_02_36.png".to_string()));
        assert!(!names.iter().any(|n| n.starts_with("Hollow Knight")));

        // 加入 Minishoot 后收集：从未识别中归位，不重复收集。
        drop(lock);
        let mini = env.add_game("Minishoot");
        let lock = env.app.repo.lock().unwrap();
        let rep = env.app.sync_shots(&lock, &mini).unwrap();
        assert_eq!(rep.added, 0);
        assert_eq!(env.app.index.shots_of(&mini.id).unwrap().len(), 1);
        let s = &env.app.index.shots_of(&mini.id).unwrap()[0];
        assert!(env.app.shot_path(s).unwrap().is_file());
        assert_eq!(env.app.index.shots_of(UNSORTED).unwrap().len(), 2);

        // 手动把 IMG_0001 归到 Hollow Knight。
        let img = env
            .app
            .index
            .shots_of(UNSORTED)
            .unwrap()
            .into_iter()
            .find(|s| s.original_path.ends_with("IMG_0001.png"))
            .unwrap();
        assert_eq!(env.app.assign_unsorted(&lock, &[img.blob], &hk).unwrap(), 1);
        assert!(
            env.app
                .index
                .shots_of(&hk.id)
                .unwrap()
                .iter()
                .any(|s| s.blob == img.blob)
        );
        assert_eq!(env.app.index.shots_of(UNSORTED).unwrap().len(), 1);

        // 关闭后不再收集。
        env.app.config.shots.keep_unsorted = false;
        put(&cap.join("Other 2026_7_19 0_02_36.png"), &png(5));
        assert!(env.app.sync_unsorted(&lock).unwrap().disabled);
    }
}

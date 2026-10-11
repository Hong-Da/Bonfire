//! 本机主存档与路线改名（U-41）。
//!
//! - 每台电脑可以为每个游戏指定一条“本机主存档”（存放在仓库的 `refs/<游戏>/primary/<电脑的键>`），
//!   仓库在几台电脑间共用时各自独立；没有设置时以 `main` 为主存档。
//! - 主存档只是“家”：备份仍接在当前路线上，不强行写进主存档；界面标出“不在主存档”，可一键回去。
//! - 任一电脑的主存档都不能直接删除，要先把别的路线设为主存档。
//! - 路线改名只换名字，快照不动；当前分支、各电脑的主存档指向旧名时一并更新。

use bonfire_core::RepoLock;
use bonfire_core::index::GameRecord;
use bonfire_core::refs::RefName;

use crate::app::App;
use crate::error::{AppError, Result};

impl App {
    /// 分支改名。
    pub fn rename_branch(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        from: &str,
        to: &str,
    ) -> Result<()> {
        let to = to.trim();
        if self
            .repo
            .read_ref(&game.id, &RefName::branch(from))?
            .is_none()
        {
            return Err(AppError::Invalid(format!("找不到分支「{from}」")));
        }
        self.repo.rename_branch(lock, &game.id, from, to)?;
        self.index
            .set_refs(&game.id, &self.repo.list_refs(&game.id)?)?;
        tracing::info!(game = %game.id, from, to, "分支改名");
        Ok(())
    }

    /// 这条路线是哪些电脑的主存档（电脑的键）。
    pub fn primary_owners(&self, game_id: &str, branch: &str) -> Result<Vec<String>> {
        Ok(self
            .repo
            .primary_branches(game_id)?
            .into_iter()
            .filter(|(_, b)| b == branch)
            .map(|(k, _)| k)
            .collect())
    }

    /// 删除分支前检查：任一电脑的主存档都不能直接删除。
    pub(crate) fn check_not_primary(&self, game_id: &str, branch: &str) -> Result<()> {
        if self.primary_owners(game_id, branch)?.is_empty() {
            Ok(())
        } else {
            Err(AppError::Invalid(format!(
                "「{branch}」是主存档，不能删除。先把别的路线设为主存档，再删除它。"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::backup::BackupOptions;
    use crate::testutil::Env;

    #[test]
    fn rename_and_primary_cannot_be_deleted() {
        let mut env = Env::new();
        let g = env.add_game("Hollow Knight");
        env.write("0/user1.dat", b"one");
        let lock = env.app.repo.lock().unwrap();
        env.app
            .backup(&lock, &g, &BackupOptions::default())
            .unwrap();
        let tip = env.app.repo.head_snapshot(&g.id).unwrap().unwrap();
        env.app
            .create_ref(
                &lock,
                &g,
                bonfire_core::refs::RefKind::Branch,
                "side",
                Some(&tip.to_hex()),
            )
            .unwrap();
        env.app
            .rename_branch(&lock, &g, "side", "main-OLD")
            .unwrap();
        let names: Vec<String> = env
            .app
            .repo
            .list_refs(&g.id)
            .unwrap()
            .into_iter()
            .map(|(r, _)| r.name)
            .collect();
        assert!(names.contains(&"main-OLD".to_string()) && !names.contains(&"side".to_string()));
        assert!(env.app.rename_branch(&lock, &g, "missing", "x").is_err());

        env.app
            .repo
            .set_primary_branch(&lock, &g.id, "pc-a", Some("main-OLD"))
            .unwrap();
        let err = env
            .app
            .delete_branch(&lock, &g, "main-OLD", true)
            .unwrap_err();
        assert!(err.to_string().contains("主存档"));
        env.app
            .repo
            .set_primary_branch(&lock, &g.id, "pc-a", None)
            .unwrap();
        assert!(env.app.delete_branch(&lock, &g, "main-OLD", true).is_ok());
    }
}

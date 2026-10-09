//! 测试辅助：在临时目录中创建应用上下文与伪造的存档目录。

use std::path::{Path, PathBuf};

use crate::{App, AppPaths, Config, Locale, NewGame};

pub struct Env {
    pub dir: tempfile::TempDir,
    pub app: App,
}

impl Env {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let app = open(dir.path());
        Self { dir, app }
    }

    pub fn reopen(&mut self) {
        self.app = open(self.dir.path());
    }

    pub fn saves(&self) -> PathBuf {
        self.dir.path().join("saves")
    }

    pub fn write(&self, rel: &str, data: &[u8]) {
        let p = self.saves().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }

    pub fn read(&self, rel: &str) -> Option<Vec<u8>> {
        std::fs::read(self.saves().join(rel)).ok()
    }

    /// 添加一个以 saves/ 为存档目录、cfg.ini 为配置文件的游戏。
    pub fn add_game(&mut self, name: &str) -> bonfire_core::index::GameRecord {
        std::fs::create_dir_all(self.saves()).unwrap();
        let cfg = self.dir.path().join("cfg.ini");
        if !cfg.exists() {
            std::fs::write(&cfg, b"fov=90").unwrap();
        }
        let lock = self.app.repo.lock().unwrap();
        self.app
            .add_game(&lock, &NewGame { name: name.into(), save_paths: vec![self.saves()], config_paths: vec![cfg], install_dir: None })
            .unwrap()
    }
}

pub fn open(root: &Path) -> App {
    let paths = AppPaths::under_root(&root.join("home"), true);
    let repo = paths.default_repo_dir.clone();
    App::open(paths, Config::default(), Locale::ZhCn, &repo).unwrap()
}

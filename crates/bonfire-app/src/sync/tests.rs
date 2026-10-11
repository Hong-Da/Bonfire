use super::*;
use crate::backup::BackupOptions;
use crate::testutil::Env;

fn folder_target(env: &Env, name: &str, content: SyncContent) -> SyncTarget {
    let dir = env.dir.path().join("remote").join(name);
    let mut t = SyncTarget::new(SyncKind::Folder, name, &dir.to_string_lossy());
    t.content = content;
    t
}

fn backup(env: &mut Env, g: &bonfire_core::index::GameRecord) {
    let lock = env.app.repo.lock().unwrap();
    env.app.backup(&lock, g, &BackupOptions::default()).unwrap();
}

#[test]
fn repo_sync_is_incremental_and_restorable() {
    let mut env = Env::new();
    env.write("a.sav", b"one");
    env.write("sub/b.sav", b"two");
    let g = env.add_game("Hollow Knight");
    backup(&mut env, &g);
    let t = env
        .app
        .save_sync_target(folder_target(&env, "硬盘", SyncContent::Repo), None)
        .unwrap();
    assert_eq!(t.id.len(), 8);

    let (plan, _, _) = env.app.sync_plan(&t).unwrap();
    assert!(plan.upload > 0 && plan.blocked.is_none());
    assert!(plan.parts.iter().any(|(k, _)| k == "settings"));
    let run = env.app.sync_push(&t.id, &mut |_, _| {}).unwrap();
    assert_eq!(run.uploaded, plan.upload);
    let remote = std::path::PathBuf::from(&t.url);
    assert!(remote.join("config.toml").is_file() && remote.join(SETTINGS_FILE).is_file());
    assert!(!remote.join("lock").exists(), "写入锁不同步");
    assert!(
        !std::fs::read_to_string(remote.join(SETTINGS_FILE))
            .unwrap()
            .contains("igdb_client_secret = \"x"),
        "设置中不含密钥"
    );

    // 再同步一次：没有变化。
    let (again, _, _) = env.app.sync_plan(&t).unwrap();
    assert_eq!((again.upload, again.delete), (0, 0));

    // 新的备份只上传新增的部分。
    env.write("a.sav", b"one more");
    backup(&mut env, &g);
    let (p2, _, _) = env.app.sync_plan(&t).unwrap();
    assert!(
        p2.upload > 0 && p2.upload < plan.upload,
        "{} vs {}",
        p2.upload,
        plan.upload
    );
    env.app.sync_push(&t.id, &mut |_, _| {}).unwrap();

    // 从远程恢复到空文件夹：得到完整、可校验的仓库。
    let dest = env.dir.path().join("restored");
    let n = env.app.sync_restore(&t.id, &dest).unwrap();
    assert!(n > 0);
    let restored = bonfire_core::Repo::open(&dest).unwrap();
    assert!(bonfire_core::fsck::fsck(&restored, true).unwrap().ok());
    assert_eq!(
        restored.head_snapshot(&g.id).unwrap(),
        env.app.repo.head_snapshot(&g.id).unwrap()
    );
    assert!(
        env.app.sync_restore(&t.id, &dest).is_err(),
        "目标文件夹不是空的"
    );
}

#[test]
fn latest_saves_with_size_limit_and_deletions() {
    let mut env = Env::new();
    env.write("small.sav", b"tiny");
    env.write("big.sav", &vec![7u8; 2 * 1024 * 1024]);
    let g = env.add_game("Celeste");
    backup(&mut env, &g);
    let mut t = folder_target(&env, "网盘", SyncContent::Latest);
    t.max_file_mb = 1;
    t.settings = false;
    let t = env.app.save_sync_target(t, None).unwrap();

    let (plan, _, _) = env.app.sync_plan(&t).unwrap();
    assert_eq!(plan.excluded.len(), 1, "超过 1 MB 的文件被排除");
    assert!(plan.excluded[0].path.ends_with("big.sav"));
    assert!(plan.excluded[0].reason.contains("1 MB"));
    assert!(plan.blocked.is_none(), "最新存档中的大文件可以排除");
    env.app.sync_push(&t.id, &mut |_, _| {}).unwrap();
    let remote = std::path::PathBuf::from(&t.url);
    let files: Vec<_> = walkdir::WalkDir::new(&remote)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let mut files = files;
    files.sort();
    assert_eq!(
        files,
        ["cfg.ini", "small.sav"],
        "存档与配置文件，不含被排除的大文件"
    );

    // 存档里删掉的文件，远程也删除。
    std::fs::remove_file(env.saves().join("small.sav")).unwrap();
    env.write("new.sav", b"n");
    backup(&mut env, &g);
    let (p2, _, _) = env.app.sync_plan(&t).unwrap();
    assert_eq!((p2.upload, p2.delete), (1, 1));
    let run = env.app.sync_push(&t.id, &mut |_, _| {}).unwrap();
    assert_eq!(run.deleted, 1);
    assert_eq!(env.app.sync_status()[0].last.as_ref().unwrap().uploaded, 1);
}

#[test]
fn total_limit_blocks_and_git_defaults() {
    let mut env = Env::new();
    // 不可压缩的数据（压缩后仍有 3 MB）。
    let mut x = 0x9E37_79B9u32;
    let data: Vec<u8> = (0..3 * 1024 * 1024)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect();
    env.write("big.sav", &data);
    let g = env.add_game("Game");
    backup(&mut env, &g);
    let mut t = folder_target(&env, "小盘", SyncContent::Repo);
    t.max_total_mb = 1;
    let t = env.app.save_sync_target(t, None).unwrap();
    let (plan, _, _) = env.app.sync_plan(&t).unwrap();
    assert_eq!(plan.level, SizeLevel::Over);
    assert!(plan.blocked.as_deref().unwrap().contains("超过总大小上限"));
    assert!(env.app.sync_push(&t.id, &mut |_, _| {}).is_err());
    assert!(
        env.app.sync_status()[0]
            .last
            .as_ref()
            .unwrap()
            .error
            .is_some(),
        "记下没有同步的原因"
    );

    // 仓库文件超过单文件上限时不能排除，整个同步被阻止。
    let mut t2 = folder_target(&env, "单文件", SyncContent::Repo);
    t2.max_file_mb = 1;
    let t2 = env.app.save_sync_target(t2, None).unwrap();
    let (p2, _, _) = env.app.sync_plan(&t2).unwrap();
    assert!(p2.blocked.as_deref().unwrap().contains("不能跳过"));

    let gh = SyncTarget::new(SyncKind::Github, "gh", "https://github.com/a/b.git");
    assert_eq!(
        (gh.max_file_mb, gh.max_total_mb, gh.shots),
        (100, 1024, false)
    );
    let ge = SyncTarget::new(SyncKind::Gitee, "ge", "https://gitee.com/a/b.git");
    assert_eq!((ge.max_file_mb, ge.max_total_mb), (50, 500));
    let mut bad = SyncTarget::new(SyncKind::Gitee, "x", "https://github.com/a/b.git");
    assert!(
        env.app.save_sync_target(bad.clone(), None).is_err(),
        "Gitee 目标要填 gitee.com 地址"
    );
    bad.name = "硬盘".into();
    bad.kind = SyncKind::Folder;
    bad.url = "D:/x".into();
    assert!(env.app.save_sync_target(bad, None).is_ok());
    let dup = folder_target(&env, "小盘", SyncContent::Repo);
    assert!(env.app.save_sync_target(dup, None).is_err(), "名称不能重复");
}

#[test]
fn shots_only_and_secrets_and_removal() {
    let mut env = Env::new();
    env.write("a.sav", b"x");
    let g = env.add_game("Game");
    backup(&mut env, &g);
    let t = env
        .app
        .save_sync_target(
            folder_target(&env, "图", SyncContent::Shots),
            Some("s3cret"),
        )
        .unwrap();
    assert_eq!(env.app.sync_secret(&t.id).as_deref(), Some("s3cret"));
    let cfg = std::fs::read_to_string(&env.app.paths.config_file).unwrap();
    assert!(
        cfg.contains("[[sync.targets]]") && !cfg.contains("s3cret"),
        "令牌不写进配置文件"
    );
    let (plan, _, _) = env.app.sync_plan(&t).unwrap();
    assert!(
        plan.parts.iter().all(|(k, _)| k != "saves"),
        "只同步截图时没有存档"
    );
    env.app.remove_sync_target(&t.id).unwrap();
    assert!(env.app.sync_secret(&t.id).is_none());
    assert!(env.app.sync_status().is_empty());
}

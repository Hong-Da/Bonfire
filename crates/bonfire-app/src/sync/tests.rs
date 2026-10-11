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
    let st = &env.app.sync_status()[0];
    assert_eq!(st.last.as_ref().unwrap().issue, Some(SyncIssue::Size));
    assert!(st.retry.is_none(), "超过上限不自动重试");
    // 忽略“超过总大小上限”后照常同步（只能忽略可以忽略的问题，其余丢弃）。
    let mut t = t;
    t.ignore = vec![SyncIssue::Size, SyncIssue::Auth, SyncIssue::Size];
    let t = env.app.save_sync_target(t, None).unwrap();
    assert_eq!(t.ignore, [SyncIssue::Size]);
    let (plan, _, _) = env.app.sync_plan(&t).unwrap();
    assert_eq!(
        (plan.level, plan.blocked.is_none()),
        (SizeLevel::Over, true)
    );
    env.app.sync_push(&t.id, &mut |_, _| {}).unwrap();

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

/// 测试用的最小 WebDAV 服务：PUT、GET、DELETE、MKCOL、PROPFIND（Depth 0 / 1），Basic 认证。
pub(crate) mod mock_dav {
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    pub struct Store {
        pub files: BTreeMap<String, Vec<u8>>,
        pub dirs: BTreeSet<String>,
    }

    /// 启动服务，返回 (地址前缀，如 http://127.0.0.1:1234, 数据)。
    pub fn start(auth: &'static str) -> (String, Arc<Mutex<Store>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let store = Arc::new(Mutex::new(Store::default()));
        store.lock().unwrap().dirs.insert("/".into());
        let s2 = store.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(conn) = conn else { continue };
                let store = s2.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(conn.try_clone().unwrap());
                    let mut out = conn;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        let mut parts = line.split_whitespace();
                        let method = parts.next().unwrap_or("").to_string();
                        let path = parts.next().unwrap_or("/").to_string();
                        let mut len = 0usize;
                        let mut depth = "0".to_string();
                        let mut authed = false;
                        loop {
                            let mut h = String::new();
                            reader.read_line(&mut h).unwrap();
                            let h = h.trim_end();
                            if h.is_empty() {
                                break;
                            }
                            let (k, v) = h.split_once(':').unwrap_or((h, ""));
                            match k.to_ascii_lowercase().as_str() {
                                "content-length" => len = v.trim().parse().unwrap_or(0),
                                "depth" => depth = v.trim().to_string(),
                                "authorization" => authed = v.trim() == auth,
                                _ => {}
                            }
                        }
                        let mut body = vec![0; len];
                        reader.read_exact(&mut body).unwrap();
                        let (status, resp) = if !authed {
                            (401, Vec::new())
                        } else {
                            handle(&store, &method, &path, &depth, body)
                        };
                        let head = format!(
                            "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nContent-Type: application/xml\r\n\r\n",
                            resp.len()
                        );
                        if out.write_all(head.as_bytes()).is_err() || out.write_all(&resp).is_err()
                        {
                            return;
                        }
                    }
                });
            }
        });
        (format!("http://{addr}"), store)
    }

    fn parent(p: &str) -> String {
        let t = p.trim_end_matches('/');
        match t.rfind('/') {
            Some(i) => t[..=i].to_string(),
            None => "/".into(),
        }
    }

    fn handle(
        store: &Arc<Mutex<Store>>,
        method: &str,
        path: &str,
        depth: &str,
        body: Vec<u8>,
    ) -> (u16, Vec<u8>) {
        let mut s = store.lock().unwrap();
        match method {
            "MKCOL" => {
                let p = format!("{}/", path.trim_end_matches('/'));
                if s.dirs.contains(&p) {
                    (405, vec![])
                } else if !s.dirs.contains(&parent(&p)) {
                    (409, vec![])
                } else {
                    s.dirs.insert(p);
                    (201, vec![])
                }
            }
            "PUT" => {
                if !s.dirs.contains(&parent(path)) {
                    return (409, vec![]);
                }
                s.files.insert(path.to_string(), body);
                (201, vec![])
            }
            "GET" => match s.files.get(path) {
                Some(b) => (200, b.clone()),
                None => (404, vec![]),
            },
            "DELETE" => {
                s.files.remove(path);
                (204, vec![])
            }
            "PROPFIND" => {
                let dir = format!("{}/", path.trim_end_matches('/'));
                if !s.dirs.contains(&dir) {
                    return (404, vec![]);
                }
                let mut xml =
                    String::from(r#"<?xml version="1.0"?><D:multistatus xmlns:D="DAV:">"#);
                xml += &format!(
                    "<D:response><D:href>{dir}</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop></D:propstat></D:response>"
                );
                if depth == "1" {
                    for d in s.dirs.iter().filter(|d| *d != &dir && parent(d) == dir) {
                        xml += &format!(
                            "<D:response><D:href>{d}</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop></D:propstat></D:response>"
                        );
                    }
                    for (f, b) in s.files.iter().filter(|(f, _)| parent(f) == dir) {
                        xml += &format!(
                            "<D:response><D:href>{f}</D:href><D:propstat><D:prop><D:resourcetype/><D:getcontentlength>{}</D:getcontentlength></D:prop></D:propstat></D:response>",
                            b.len()
                        );
                    }
                }
                xml += "</D:multistatus>";
                (207, xml.into_bytes())
            }
            _ => (405, vec![]),
        }
    }
}

#[test]
fn webdav_sync_and_restore() {
    let (url, store) = mock_dav::start("Basic dXNlcjpwYXNz");
    let mut env = Env::new();
    env.write("存档 1.sav", b"one");
    let g = env.add_game("空洞骑士");
    backup(&mut env, &g);
    let mut t = SyncTarget::new(SyncKind::Webdav, "坚果云", &format!("{url}/dav/篝火 备份"));
    t.user = "user".into();
    let t = env.app.save_sync_target(t.clone(), Some("wrong")).unwrap();
    assert!(env.app.sync_test(&t).is_err(), "密码不对");
    env.app.set_sync_secret(&t.id, "pass").unwrap();
    assert!(env.app.sync_test(&t).unwrap().contains("可以写入"));

    let run = env.app.sync_push(&t.id, &mut |_, _| {}).unwrap();
    assert!(run.uploaded > 3 && run.error.is_none());
    {
        let s = store.lock().unwrap();
        assert!(s.files.keys().any(|k| k.ends_with("/config.toml")));
        assert!(
            s.files
                .keys()
                .all(|k| k.starts_with("/dav/%E7%AF%9D%E7%81%AB%20%E5%A4%87%E4%BB%BD/")),
            "中文路径已编码"
        );
    }
    let dest = env.dir.path().join("from-dav");
    let n = env.app.sync_restore(&t.id, &dest).unwrap();
    assert_eq!(n, store.lock().unwrap().files.len());
    let restored = bonfire_core::Repo::open(&dest).unwrap();
    assert!(bonfire_core::fsck::fsck(&restored, true).unwrap().ok());
}

/// 远程裸仓库中分支上的文件。
fn bare_files(path: &std::path::Path, branch: &str) -> Vec<String> {
    let repo = git2::Repository::open_bare(path).unwrap();
    let Ok(r) = repo.find_reference(&format!("refs/heads/{branch}")) else {
        return Vec::new();
    };
    let tree = r.peel_to_commit().unwrap().tree().unwrap();
    let mut out = Vec::new();
    tree.walk(git2::TreeWalkMode::PreOrder, |dir, e| {
        if e.kind() == Some(git2::ObjectType::Blob) {
            out.push(format!("{dir}{}", e.name().unwrap()));
        }
        git2::TreeWalkResult::Ok
    })
    .unwrap();
    out
}

fn commit_count(path: &std::path::Path, branch: &str) -> usize {
    let repo = git2::Repository::open_bare(path).unwrap();
    let mut walk = repo.revwalk().unwrap();
    walk.push_ref(&format!("refs/heads/{branch}")).unwrap();
    walk.count()
}

#[test]
fn git_sync_push_restore_and_compact() {
    let mut env = Env::new();
    env.write("a.sav", b"one");
    let g = env.add_game("Hades");
    backup(&mut env, &g);
    let bare = env.dir.path().join("remote.git");
    git2::Repository::init_bare(&bare).unwrap();
    let mut t = SyncTarget::new(SyncKind::Gitee, "码云", &bare.to_string_lossy());
    t.branch = "saves".into();
    let t = env.app.save_sync_target(t, None).unwrap();
    assert_eq!((t.max_file_mb, t.max_total_mb, t.shots), (50, 500, false));
    assert!(env.app.sync_test(&t).unwrap().contains("还没有内容"));

    let run = env.app.sync_push(&t.id, &mut |_, _| {}).unwrap();
    assert!(run.error.is_none() && run.uploaded > 0);
    let files = bare_files(&bare, "saves");
    assert!(files.contains(&"config.toml".to_string()));
    assert!(files.iter().any(|f| f.starts_with("objects/")));
    assert!(files.contains(&SETTINGS_FILE.to_string()));
    assert_eq!(commit_count(&bare, "saves"), 1);

    // 增量：新的备份只提交新增的文件。
    env.write("a.sav", b"two");
    backup(&mut env, &g);
    let (p2, _, _) = env.app.sync_plan(&t).unwrap();
    assert!(
        p2.remote_bytes > p2.upload_bytes,
        "Git 远程含历史：已推送的加上新增的"
    );
    env.app.sync_push(&t.id, &mut |_, _| {}).unwrap();
    assert_eq!(commit_count(&bare, "saves"), 2);

    // 另一台电脑：缓存不同（换一个目标 ID），从远程恢复完整仓库。
    let dest = env.dir.path().join("from-git");
    std::fs::remove_dir_all(env.app.paths.cache_dir.join("sync")).unwrap();
    let n = env.app.sync_restore(&t.id, &dest).unwrap();
    assert_eq!(n, bare_files(&bare, "saves").len());
    let restored = bonfire_core::Repo::open(&dest).unwrap();
    assert!(bonfire_core::fsck::fsck(&restored, true).unwrap().ok());

    // 压缩远程：只剩一个提交，内容不变。
    let before = bare_files(&bare, "saves");
    env.app.sync_compact(&t.id).unwrap();
    assert_eq!(commit_count(&bare, "saves"), 1);
    assert_eq!(bare_files(&bare, "saves"), before);
}

#[test]
fn schedules_and_settings_apply() {
    let mut env = Env::new();
    let mut a = folder_target(&env, "每次备份后", SyncContent::Repo);
    a.schedule = SyncSchedule::AfterBackup;
    let a = env.app.save_sync_target(a, None).unwrap();
    let mut b = folder_target(&env, "每 6 小时", SyncContent::Repo);
    b.schedule = SyncSchedule::Hours;
    b.every_hours = 6;
    let b = env.app.save_sync_target(b, None).unwrap();
    assert_eq!(
        env.app.sync_after_backup_targets(),
        std::slice::from_ref(&a.id)
    );
    let now = jiff::Zoned::now();
    assert_eq!(
        env.app.sync_due(&now),
        std::slice::from_ref(&b.id),
        "从没同步过：到期"
    );
    env.app.sync_push(&b.id, &mut |_, _| {}).unwrap();
    assert!(env.app.sync_due(&now).is_empty(), "刚同步过：没到期");

    // 套用远程的设置：保留本机的仓库位置与密钥。
    env.app.config.metadata.steamgriddb_api_key = "local-key".into();
    let mut remote = env.app.config.clone();
    remote.auto_backup.on_exit = true;
    remote.general.repo = Some("Z:/elsewhere".into());
    remote.metadata.steamgriddb_api_key.clear();
    let text = format!("# 篝火设置\n\n{}", toml::to_string_pretty(&remote).unwrap());
    let repo_before = env.app.config.general.repo.clone();
    env.app.apply_portable_settings(&text).unwrap();
    assert!(env.app.config.auto_backup.on_exit);
    assert_eq!(env.app.config.general.repo, repo_before);
    assert_eq!(env.app.config.metadata.steamgriddb_api_key, "local-key");
    assert!(env.app.apply_portable_settings("not = [toml").is_err());
}

#[test]
fn network_failures_retry_with_backoff() {
    let mut env = Env::new();
    env.write("a.sav", b"one");
    let g = env.add_game("Game");
    backup(&mut env, &g);
    // 目标文件夹建不出来（路径中有同名文件），相当于移动硬盘没连上。
    let blocker = env.dir.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    let mut t = SyncTarget::new(
        SyncKind::Folder,
        "硬盘",
        &blocker.join("sub").to_string_lossy(),
    );
    t.schedule = SyncSchedule::AfterBackup;
    let t = env.app.save_sync_target(t, None).unwrap();
    let now = jiff::Zoned::now();
    let later = |m: i64| now.checked_add(jiff::SignedDuration::from_mins(m)).unwrap();
    for (attempt, mins) in RETRY_MINUTES.iter().enumerate() {
        assert!(env.app.sync_push(&t.id, &mut |_, _| {}).is_err());
        let st = env.app.sync_status().remove(0);
        assert_eq!(st.last.unwrap().issue, Some(SyncIssue::Network));
        let r = st.retry.expect("连不上目标时自动重试");
        assert_eq!(r.attempt as usize, attempt + 1);
        assert!(
            env.app.sync_due(&later(mins - 1)).is_empty(),
            "还没到重试时间"
        );
        assert_eq!(
            env.app.sync_due(&later(mins + 1)),
            std::slice::from_ref(&t.id)
        );
    }
    // 第 4 次失败后不再重试，等下一次备份。
    assert!(env.app.sync_push(&t.id, &mut |_, _| {}).is_err());
    assert!(env.app.sync_status()[0].retry.is_none());

    // 修好后同步成功，重试记录清除；手动同步的目标失败不自动重试。
    std::fs::remove_file(&blocker).unwrap();
    env.app.sync_push(&t.id, &mut |_, _| {}).unwrap();
    assert!(env.app.sync_status()[0].retry.is_none());
    let mut m = t.clone();
    m.schedule = SyncSchedule::Manual;
    assert!(next_retry(&m, SyncIssue::Network, None, jiff::Timestamp::now()).is_none());
    assert!(
        next_retry(&t, SyncIssue::Auth, None, jiff::Timestamp::now()).is_none(),
        "令牌错误重试也没用"
    );
    assert!(SyncIssue::Size.ignorable() && !SyncIssue::Public.ignorable());
}

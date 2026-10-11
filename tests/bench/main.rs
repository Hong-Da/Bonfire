//! 性能与资源基准（M4-5，文档 10.1）。数据都在临时目录中按固定种子生成，用完删除。
//!
//! ```bat
//! cargo test -p bonfire-tests --release --test bench -- --ignored --nocapture --test-threads 1
//! ```
//!
//! 指标：200 款游戏无变化时增量扫描 < 3 秒、首次全量扫描 < 30 秒；哈希 + 压缩吞吐 ≥ 200 MB/s；
//! 快照元数据每个 < 10 KB。只在 release 构建中断言（debug 构建只打印结果）。
//! 托盘常驻内存另用安装版测量，见 docs/acceptance/M4-acceptance.md。

use std::path::{Path, PathBuf};
use std::time::Instant;

use bonfire_app::core::Kind;
use bonfire_app::{App, AppPaths, BackupOptions, Config, Locale, NewGame};

const MB: f64 = 1024.0 * 1024.0;

/// 固定种子的伪随机字节（不可压缩，模拟加密或压缩过的存档）。
fn random_bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.truncate(len);
    out
}

fn open(root: &Path) -> App {
    let paths = AppPaths::under_root(&root.join("home"), true);
    let repo = paths.default_repo_dir.clone();
    App::open(paths, Config::default(), Locale::ZhCn, &repo).unwrap()
}

fn write(p: &Path, data: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, data).unwrap();
}

fn check(name: &str, ok: bool, detail: String) {
    println!("{} {name}：{detail}", if ok { "通过" } else { "未达标" });
    if cfg!(not(debug_assertions)) {
        assert!(ok, "{name}：{detail}");
    }
}

fn add(app: &mut App, name: &str, dir: PathBuf) -> bonfire_app::core::index::GameRecord {
    let lock = app.repo.lock().unwrap();
    app.add_game(
        &lock,
        &NewGame {
            name: name.into(),
            save_paths: vec![dir],
            config_paths: Vec::new(),
            install_dir: None,
        },
    )
    .unwrap()
}

#[test]
#[ignore = "基准测试：用 --release --ignored 运行"]
fn scan_200_games() {
    let d = tempfile::tempdir().unwrap();
    let mut app = open(d.path());
    // 200 款游戏，每款 5 个存档文件（4 KB–200 KB），共约 1000 个文件。
    let mut games = Vec::new();
    for g in 0..200u64 {
        let dir = d.path().join("saves").join(format!("Game {g:03}"));
        for f in 0..5u64 {
            let size = 4096 + ((g * 7 + f * 13) % 50) as usize * 4096;
            write(
                &dir.join(format!("slot{f}.sav")),
                &random_bytes(g * 100 + f, size),
            );
        }
        games.push(add(&mut app, &format!("Game {g:03}"), dir));
    }

    let lock = app.repo.lock().unwrap();
    let t = Instant::now();
    for g in &games {
        app.backup(&lock, g, &BackupOptions::default()).unwrap();
    }
    println!("首次备份 200 款游戏：{:.2} 秒", t.elapsed().as_secs_f64());
    drop(lock);

    // 全量扫描：不用修改时间判断，每个文件都重新计算哈希再与快照比较（界面“检查更新”的完整校验）。
    let t = Instant::now();
    for g in &games {
        assert!(!app.status_verified(g).unwrap().summary.has_changes());
    }
    let first = t.elapsed().as_secs_f64();
    check(
        "全量扫描（200 款游戏，没有缓存，校验全部文件）",
        first < 30.0,
        format!("{first:.2} 秒（目标 < 30 秒）"),
    );

    // 增量扫描：无变化，只比较大小与修改时间。
    let t = Instant::now();
    for g in &games {
        let s = app.status(g).unwrap();
        assert!(!s.summary.has_changes());
    }
    let inc = t.elapsed().as_secs_f64();
    check(
        "增量扫描（200 款游戏，无变化）",
        inc < 3.0,
        format!("{inc:.2} 秒（目标 < 3 秒）"),
    );

    // 快照元数据：快照对象 + 清单对象（压缩后）。
    let mut sizes = Vec::new();
    for g in &games {
        let id = app.repo.head_snapshot(&g.id).unwrap().unwrap();
        let snap = app.repo.get_snapshot(&id).unwrap();
        let bytes = app
            .repo
            .read_object_file(Kind::Snapshot, &id)
            .unwrap()
            .len()
            + app
                .repo
                .read_object_file(Kind::Tree, &snap.tree)
                .unwrap()
                .len();
        sizes.push(bytes);
    }
    let max = *sizes.iter().max().unwrap();
    let avg = sizes.iter().sum::<usize>() / sizes.len();
    check(
        "快照元数据",
        max < 10 * 1024,
        format!("平均 {avg} 字节，最大 {max} 字节（5 个文件；目标 < 10 KB）"),
    );
}

#[test]
#[ignore = "基准测试：用 --release --ignored 运行"]
fn backup_throughput() {
    let d = tempfile::tempdir().unwrap();
    let mut app = open(d.path());
    // 多个文件：64 个 8 MB 的不可压缩文件（512 MB）。
    let dir = d.path().join("saves").join("Big");
    for i in 0..64u64 {
        write(
            &dir.join(format!("part{i:02}.bin")),
            &random_bytes(i + 1, 8 << 20),
        );
    }
    let g = add(&mut app, "Big", dir.clone());
    let one = d.path().join("saves").join("One");
    write(&one.join("world.bin"), &random_bytes(99, 256 << 20));
    let g1 = add(&mut app, "One", one);
    let lock = app.repo.lock().unwrap();
    let t = Instant::now();
    app.backup(&lock, &g, &BackupOptions::default()).unwrap();
    let secs = t.elapsed().as_secs_f64();
    let rate = 512.0 / secs;
    check(
        "备份吞吐（64 个文件，512 MB）",
        rate >= 200.0,
        format!("{secs:.2} 秒，{rate:.0} MB/s（目标 ≥ 200 MB/s）"),
    );

    // 单个大文件：分块串行处理，只打印供参考。
    let g = g1;
    let t = Instant::now();
    app.backup(&lock, &g, &BackupOptions::default()).unwrap();
    let secs = t.elapsed().as_secs_f64();
    println!(
        "参考 单个 256 MB 文件：{secs:.2} 秒，{:.0} MB/s",
        256.0 / secs
    );

    // 未变化时再次备份：零新增存储。
    let before: u64 = dir_size(&app.repo.objects_dir());
    let r = app
        .backup(
            &lock,
            &g,
            &BackupOptions {
                force: true,
                ..Default::default()
            },
        )
        .unwrap();
    let after: u64 = dir_size(&app.repo.objects_dir());
    let added = after.saturating_sub(before);
    check(
        "未变化文件零新增存储",
        added < 64 * 1024,
        format!(
            "强制再备份一次新增 {:.1} KB（只有快照与清单）",
            added as f64 / 1024.0
        ),
    );
    assert!(matches!(r, bonfire_app::BackupResult::Created { .. }));
    println!("仓库大小 {:.0} MB", dir_size(app.repo.root()) as f64 / MB);
}

fn dir_size(p: &Path) -> u64 {
    let mut n = 0;
    for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
        let m = e.metadata().unwrap();
        n += if m.is_dir() {
            dir_size(&e.path())
        } else {
            m.len()
        };
    }
    n
}

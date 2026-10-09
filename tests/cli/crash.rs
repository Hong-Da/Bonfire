//! 崩溃一致性测试（M1 验收：随机断电不损坏已有快照）。
//!
//! 通过 `BONFIRE_FAULT_INJECT` 让 bonfire 子进程在写入流程的指定位置立即中止
//! （`process::abort`，不执行任何清理，等同于强杀或断电），然后检查：
//! - `fsck` 无错误；
//! - 中止前的历史完好，HEAD 仍指向中止前的快照；
//! - 再次备份成功，且能把旧快照逐字节恢复出来。

#[path = "../common.rs"]
mod common;

use common::Sandbox;

/// 固定种子的伪随机数据（xorshift64）。
fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.max(1);
    let mut v = Vec::with_capacity(len + 8);
    while v.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.truncate(len);
    v
}

struct Fixture {
    sb: Sandbox,
    v1: Vec<(String, Vec<u8>)>,
    v2: Vec<(String, Vec<u8>)>,
}

impl Fixture {
    /// v1：一个 6 MiB 的大存档（会分块）+ 两个小文件，已备份。v2：修改大文件中间、修改一个小文件、新增一个文件（尚未备份）。
    fn new() -> Self {
        let sb = Sandbox::new();
        let big = pseudo_random(6 << 20, 7);
        let v1 = vec![
            ("big.sav".to_string(), big.clone()),
            ("slot1.sav".to_string(), b"level 1".to_vec()),
            ("子目录/存档 ã.sav".to_string(), b"chapter".to_vec()),
        ];
        for (p, d) in &v1 {
            sb.write(p, d);
        }
        sb.ok(&["game", "add", "G", "--path", "{saves}"]);
        sb.ok(&["backup", "G", "-m", "v1"]);

        let mut big2 = big;
        for b in &mut big2[3 << 20..(3 << 20) + 4096] {
            *b ^= 0x5a;
        }
        let v2 = vec![
            ("big.sav".to_string(), big2),
            ("slot1.sav".to_string(), b"level 2".to_vec()),
            ("子目录/存档 ã.sav".to_string(), b"chapter".to_vec()),
            ("new.sav".to_string(), b"new file".to_vec()),
        ];
        for (p, d) in &v2 {
            sb.write(p, d);
        }
        Self { sb, v1, v2 }
    }

    fn head(&self) -> String {
        self.sb.json(&["log", "G", "--limit", "1"])[0]["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn snapshot_count(&self) -> usize {
        self.sb.json(&["log", "G"]).as_array().unwrap().len()
    }

    /// 以注入的故障执行一条命令，要求进程异常结束。
    fn crash(&self, point: &str, args: &[&str]) {
        let args: Vec<String> = args.iter().map(|a| self.sb.expand(a)).collect();
        let out = self
            .sb
            .command()
            .env("BONFIRE_FAULT_INJECT", point)
            .args(&args)
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "注入 {point} 后进程应异常结束：{}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("fault-inject"),
            "未命中注入点 {point}：{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn assert_fsck_ok(&self, ctx: &str) {
        let r = self.sb.json(&["fsck", "--deep"]);
        assert_eq!(
            r["ok"], true,
            "{ctx}：fsck 报告问题 {:#}",
            r["report"]["errors"]
        );
    }

    fn assert_files(&self, expected: &[(String, Vec<u8>)], ctx: &str) {
        for (p, d) in expected {
            assert!(
                self.sb.read(p).as_deref() == Some(d.as_slice()),
                "{ctx}：{p} 内容不符"
            );
        }
    }
}

/// 备份过程中各个写入点崩溃。
#[test]
fn crash_during_backup_never_corrupts_history() {
    let points = [
        "before-rename:1",
        "before-rename:3",
        "after-chunk:1",
        "after-chunk:2",
        "after-blob:1",
        "after-blob:2",
        "after-tree",
        "after-snapshot",
        // 引用文件与 reflog 写入中（快照已写，更新分支指针的临时文件已落盘）。
        "before-rename:7",
    ];
    for point in points {
        let f = Fixture::new();
        let before = f.head();
        f.crash(point, &["backup", "G", "-m", "v2"]);

        f.assert_fsck_ok(point);
        let head = f.head();
        // 分支要么还指向 v1，要么（指针已经原子更新）指向完整的 v2，不会是半个快照。
        if head != before {
            assert_eq!(f.snapshot_count(), 2, "{point}");
        }

        // 再次备份成功；旧快照可逐字节恢复。
        f.sb.ok(&["backup", "G", "-m", "v2 retry"]);
        f.assert_fsck_ok(point);
        let first = f.sb.json(&["log", "G"]).as_array().unwrap().last().unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        f.sb.ok(&["restore", "G", &first, "--full", "-y"]);
        f.assert_files(&f.v1, point);
        assert!(
            f.sb.read("new.sav").is_none(),
            "{point}：完全恢复应删除 v1 中没有的文件"
        );
    }
}

/// 恢复过程中替换了部分文件后崩溃：撤销恢复能回到恢复前的内容。
#[test]
fn crash_during_restore_can_be_undone() {
    for nth in [1, 2] {
        let f = Fixture::new();
        // v2 尚未备份：恢复会先建保护快照。
        f.crash(
            &format!("restore-mid-commit:{nth}"),
            &["restore", "G", "HEAD", "--full", "-y"],
        );
        f.assert_fsck_ok("restore crash");
        f.sb.ok(&["undo-restore", "G", "-y"]);
        f.assert_files(&f.v2, &format!("undo after restore-mid-commit:{nth}"));
        f.assert_fsck_ok("after undo");
        let st = f.sb.json(&["status", "G"]);
        assert_eq!(st["state"], "up-to-date", "{st:#}");
    }
}

/// 更多随机位置的压力测试（较慢，手动运行：cargo test -p bonfire-tests --test crash -- --ignored）。
#[test]
#[ignore]
fn crash_stress() {
    for n in 1..=12 {
        for point in ["after-chunk", "before-rename"] {
            let f = Fixture::new();
            let p = format!("{point}:{n}");
            let args = ["backup", "G"];
            let out =
                f.sb.command()
                    .env("BONFIRE_FAULT_INJECT", &p)
                    .args(args)
                    .output()
                    .unwrap();
            if out.status.success() {
                continue; // 写入次数少于 n，未命中。
            }
            f.assert_fsck_ok(&p);
            f.sb.ok(&["backup", "G"]);
            f.assert_fsck_ok(&p);
        }
    }
}

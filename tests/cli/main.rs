//! CLI 测试：执行真实的 bonfire 二进制，按 tests/cases/cli_cases.toml 逐条检查。

#[path = "../common.rs"]
mod common;

use common::{CliCase, Sandbox, load_cli_cases};

fn run_case(case: &CliCase) -> Result<(), String> {
    let sb = Sandbox::new();
    if let Some(f) = &case.fixture {
        sb.fixture(f);
    }
    if let Some(r) = &case.root {
        sb.root_fixture(r);
    }
    for pre in &case.pre {
        let args: Vec<&str> = pre.iter().map(String::as_str).collect();
        let out = sb.run(&args);
        if !out.status.success() {
            return Err(format!("前置命令 {pre:?} 失败：{}", String::from_utf8_lossy(&out.stderr)));
        }
    }
    for (rel, content) in &case.edit {
        sb.write(rel, content.as_bytes());
    }
    for p in &case.remove {
        let p = std::path::PathBuf::from(sb.expand(p));
        let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
    }
    let args: Vec<&str> = case.args.iter().map(String::as_str).collect();
    let out = sb.run(&args);
    let stdout = String::from_utf8(out.stdout).map_err(|_| "stdout 不是 UTF-8".to_string())?;
    let stderr = String::from_utf8(out.stderr).map_err(|_| "stderr 不是 UTF-8".to_string())?;
    let code = out.status.code().unwrap_or(-1);
    let ctx = || format!("\n退出码：{code}\nstdout：{stdout}\nstderr：{stderr}");

    if code != case.expect_exit {
        return Err(format!("退出码应为 {}{}", case.expect_exit, ctx()));
    }
    for s in &case.stdout_contains {
        if !stdout.contains(sb.expand(s).as_str()) {
            return Err(format!("stdout 应包含 {s:?}{}", ctx()));
        }
    }
    for s in &case.stdout_not_contains {
        if stdout.contains(sb.expand(s).as_str()) {
            return Err(format!("stdout 不应包含 {s:?}{}", ctx()));
        }
    }
    for s in &case.stderr_contains {
        if !stderr.contains(sb.expand(s).as_str()) {
            return Err(format!("stderr 应包含 {s:?}{}", ctx()));
        }
    }
    if code != 0 && stderr.trim().is_empty() {
        return Err(format!("出错时 stderr 应有提示{}", ctx()));
    }
    common::check_json("stdout", &stdout, &case.expect_stdout_json).map_err(|e| e + &ctx())?;
    common::check_json("stderr", &stderr, &case.expect_stderr_json).map_err(|e| e + &ctx())?;
    for (rel, want) in &case.expect_files {
        let got = sb.read(rel).map(|b| String::from_utf8_lossy(&b).into_owned());
        if got.as_deref() != Some(want.as_str()) {
            return Err(format!("文件 {rel} 应为 {want:?}，实际为 {got:?}{}", ctx()));
        }
    }
    Ok(())
}

#[test]
fn cli_cases() {
    let cases = load_cli_cases();
    assert!(!cases.is_empty());
    let failures: Vec<String> = cases.iter().filter_map(|c| run_case(c).err().map(|e| format!("[{}] {e}", c.id))).collect();
    assert!(failures.is_empty(), "{} 条 CLI 用例失败：\n{}", failures.len(), failures.join("\n\n"));
}

#[test]
fn case_ids_are_unique() {
    let cases = load_cli_cases();
    let mut ids: Vec<_> = cases.iter().map(|c| c.id.as_str()).collect();
    ids.sort();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "cli_cases.toml 中有重复的 id");
}

/// 每个子命令至少有一条正常用例和一条错误用例。
#[test]
fn every_command_has_ok_and_error_cases() {
    let cases = load_cli_cases();
    let cmds = [
        "game add", "game list", "game rm", "game enable", "game confirm", "scan", "status", "info", "rules info", "backup", "log", "diff", "restore", "undo-restore", "comment", "branch", "tag",
        "profile save", "profile list", "profile rm", "fsck", "reindex",
    ];
    let mut missing = Vec::new();
    for cmd in cmds {
        let words: Vec<&str> = cmd.split(' ').collect();
        let matching: Vec<&CliCase> = cases
            .iter()
            .filter(|c| {
                let a = strip_globals(&c.args);
                a.len() >= words.len() && a[..words.len()] == words[..]
            })
            .collect();
        if !matching.iter().any(|c| c.expect_exit == 0) {
            missing.push(format!("{cmd}：缺少正常用例"));
        }
        if !matching.iter().any(|c| c.expect_exit != 0) {
            missing.push(format!("{cmd}：缺少错误用例"));
        }
    }
    assert!(missing.is_empty(), "{missing:#?}");
}

/// 去掉开头的全局参数（含其取值），得到子命令及其参数。
fn strip_globals(args: &[String]) -> Vec<&str> {
    let mut out = Vec::new();
    let mut it = args.iter().map(String::as_str);
    while let Some(a) = it.next() {
        match a {
            "--repo" | "--config" | "--lang" | "--output" => {
                it.next();
            }
            "--json" | "-q" | "--quiet" | "-v" | "--verbose" | "-y" | "--yes" => {}
            _ => out.push(a),
        }
    }
    out
}

#[test]
fn repo_locked_by_other_process_exits_6() {
    let sb = Sandbox::new();
    sb.fixture("basic");
    sb.ok(&["game", "add", "G", "--path", "{saves}"]);
    let repo = bonfire_app::core::Repo::open(&sb.repo()).unwrap();
    let _lock = repo.lock().unwrap();
    let out = sb.run(&["backup", "G", "--json"]);
    assert_eq!(out.status.code(), Some(6), "{}", String::from_utf8_lossy(&out.stderr));
    let err: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(err["error"]["kind"], "locked");
}

#[test]
fn help_switches_language() {
    let sb = Sandbox::new();
    let zh = String::from_utf8(sb.command().args(["--help"]).output().unwrap().stdout).unwrap();
    let en = String::from_utf8(sb.command().args(["--lang", "en", "backup", "--help"]).output().unwrap().stdout).unwrap();
    assert!(zh.contains("备份游戏，生成快照"), "{zh}");
    assert!(en.contains("Back up games as snapshots"), "{en}");
    assert!(en.contains("Snapshot comment"), "{en}");
}

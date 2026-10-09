//! CLI 测试：执行真实的 bonfire 二进制，按 tests/cases/cli_cases.toml 逐条检查。

#[path = "../common.rs"]
mod common;

use common::{CliCase, load_cli_cases};

fn run_case(case: &CliCase) -> Result<(), String> {
    let home = tempfile::tempdir().map_err(|e| e.to_string())?;
    let out = common::bonfire()
        .args(case.expanded_args())
        .env(bonfire_app::paths::ENV_HOME, home.path())
        .env_remove(bonfire_app::paths::ENV_CONFIG)
        .env_remove(bonfire_app::config::ENV_LANG)
        .env_remove(bonfire_app::config::ENV_REPO)
        .env_remove(bonfire_app::config::ENV_LOG_LEVEL)
        .output()
        .map_err(|e| format!("无法执行 bonfire：{e}"))?;
    let stdout = String::from_utf8(out.stdout).map_err(|_| "stdout 不是 UTF-8".to_string())?;
    let stderr = String::from_utf8(out.stderr).map_err(|_| "stderr 不是 UTF-8".to_string())?;
    let code = out.status.code().unwrap_or(-1);
    let ctx = || format!("\n退出码：{code}\nstdout：{stdout}\nstderr：{stderr}");

    if code != case.expect_exit {
        return Err(format!("退出码应为 {}{}", case.expect_exit, ctx()));
    }
    for s in &case.stdout_contains {
        if !stdout.contains(s.as_str()) {
            return Err(format!("stdout 应包含 {s:?}{}", ctx()));
        }
    }
    for s in &case.stderr_contains {
        if !stderr.contains(s.as_str()) {
            return Err(format!("stderr 应包含 {s:?}{}", ctx()));
        }
    }
    if code != 0 && stderr.trim().is_empty() {
        return Err(format!("出错时 stderr 应有提示{}", ctx()));
    }
    common::check_json("stdout", &stdout, &case.expect_stdout_json).map_err(|e| e + &ctx())?;
    common::check_json("stderr", &stderr, &case.expect_stderr_json).map_err(|e| e + &ctx())?;
    Ok(())
}

#[test]
fn cli_cases() {
    let cases = load_cli_cases();
    assert!(!cases.is_empty());
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|c| run_case(c).err().map(|e| format!("[{}] {e}", c.id)))
        .collect();
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

//! 测试公共代码：定位工作区与 bonfire 二进制、读取用例数据。
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// 工作区根目录。
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

pub fn inputs_dir() -> PathBuf {
    root().join("tests").join("inputs")
}

/// `cargo test --workspace` 会先构建所有二进制，因此直接从 target 目录取。
pub fn bonfire_path() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| root().join("target"));
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    let p = target.join(profile).join(format!("bonfire{}", std::env::consts::EXE_SUFFIX));
    assert!(p.is_file(), "找不到 {}，请用 cargo test --workspace 运行（会先构建 bonfire）", p.display());
    p
}

pub fn bonfire() -> std::process::Command {
    std::process::Command::new(bonfire_path())
}

#[derive(Debug, Deserialize)]
pub struct CliCases {
    pub case: Vec<CliCase>,
}

#[derive(Debug, Deserialize)]
pub struct CliCase {
    pub id: String,
    pub title: BTreeMap<String, String>,
    pub args: Vec<String>,
    pub expect_exit: i32,
    #[serde(default)]
    pub stdout_contains: Vec<String>,
    #[serde(default)]
    pub stderr_contains: Vec<String>,
    #[serde(default)]
    pub expect_stdout_json: BTreeMap<String, toml::Value>,
    #[serde(default)]
    pub expect_stderr_json: BTreeMap<String, toml::Value>,
    #[serde(default)]
    pub in_docs: bool,
}

impl CliCase {
    pub fn expanded_args(&self) -> Vec<String> {
        let inputs = inputs_dir().to_string_lossy().replace('\\', "/");
        self.args.iter().map(|a| a.replace("{inputs}", &inputs)).collect()
    }

    /// 文档中展示的命令行（占位符换成相对路径）。
    pub fn doc_command(&self) -> String {
        let args: Vec<String> = self
            .args
            .iter()
            .map(|a| {
                let a = a.replace("{inputs}", "tests/inputs");
                if a.contains(' ') { format!("\"{a}\"") } else { a }
            })
            .collect();
        format!("bonfire {}", args.join(" "))
    }
}

pub fn load_cli_cases() -> Vec<CliCase> {
    let p = root().join("tests/cases/cli_cases.toml");
    let text = std::fs::read_to_string(&p).expect("读取 cli_cases.toml");
    toml::from_str::<CliCases>(&text).expect("cli_cases.toml 格式错误").case
}

/// 按 `a.b.c` 字段路径检查 JSON 输出（取最后一个非空行解析）。
pub fn check_json(stream: &str, text: &str, expect: &BTreeMap<String, toml::Value>) -> Result<(), String> {
    if expect.is_empty() {
        return Ok(());
    }
    let line = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
    let v: serde_json::Value = serde_json::from_str(line).map_err(|e| format!("{stream} 不是 JSON（{e}）"))?;
    for (path, want) in expect {
        let got = path.split('.').try_fold(&v, |cur, k| cur.get(k));
        let want_json = serde_json::to_value(want).unwrap();
        if got != Some(&want_json) {
            return Err(format!("{stream} 的 {path} 应为 {want_json}，实际为 {got:?}"));
        }
    }
    Ok(())
}

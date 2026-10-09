//! 测试公共代码：定位工作区与 bonfire 二进制、读取与执行 CLI 用例。
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde::Deserialize;

/// 工作区根目录。
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

pub fn inputs_dir() -> PathBuf {
    root().join("tests").join("inputs")
}

/// `cargo test --workspace` 会先构建所有二进制，因此直接从 target 目录取。
pub fn bonfire_path() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("target"));
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let p = target
        .join(profile)
        .join(format!("bonfire{}", std::env::consts::EXE_SUFFIX));
    assert!(
        p.is_file(),
        "找不到 {}，请用 cargo test --workspace 运行（会先构建 bonfire）",
        p.display()
    );
    p
}

/// 隔离的运行环境：临时的 BONFIRE_HOME 与存档目录，不读写本机真实配置与仓库。
pub struct Sandbox {
    pub dir: tempfile::TempDir,
}

impl Sandbox {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    pub fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    pub fn saves(&self) -> PathBuf {
        self.dir.path().join("saves")
    }

    pub fn repo(&self) -> PathBuf {
        self.home().join("repo")
    }

    /// 把 tests/inputs/saves/<名称> 复制为存档目录。
    pub fn fixture(&self, name: &str) {
        copy_dir(&inputs_dir().join("saves").join(name), &self.saves());
    }

    /// 把 tests/inputs/roots/<名称> 整体复制到沙箱根目录（含伪造的平台 pc/ 与配置 home/）。
    pub fn root_fixture(&self, name: &str) {
        copy_dir(&inputs_dir().join("roots").join(name), self.dir.path());
    }

    pub fn write(&self, rel: &str, data: &[u8]) {
        let p = self.saves().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }

    pub fn read(&self, rel: &str) -> Option<Vec<u8>> {
        std::fs::read(self.saves().join(rel)).ok()
    }

    pub fn expand(&self, s: &str) -> String {
        let slash = |p: PathBuf| p.to_string_lossy().replace('\\', "/");
        s.replace("{inputs}", &slash(inputs_dir()))
            .replace("{saves}", &slash(self.saves()))
            .replace("{tmp}", &slash(self.dir.path().to_path_buf()))
    }

    pub fn command(&self) -> std::process::Command {
        let mut c = std::process::Command::new(bonfire_path());
        c.env("BONFIRE_HOME", self.home())
            .env_remove("BONFIRE_CONFIG")
            .env_remove("BONFIRE_LANG")
            .env_remove("BONFIRE_REPO")
            .env_remove("BONFIRE_LOG_LEVEL")
            .env_remove("BONFIRE_FAULT_INJECT")
            // 测试中的语言固定为中文，不随开发机系统语言变化。
            .env("BONFIRE_LANG", "zh-CN")
            // 不读取开发机上真实的平台安装，也不联网。
            .env("BONFIRE_FAKE_ROOT", self.dir.path().join("pc"))
            .env("BONFIRE_OFFLINE", "1")
            .stdin(std::process::Stdio::null());
        c
    }

    pub fn run(&self, args: &[&str]) -> Output {
        let args: Vec<String> = args.iter().map(|a| self.expand(a)).collect();
        self.command()
            .args(&args)
            .output()
            .expect("无法执行 bonfire")
    }

    /// 执行并要求成功，返回 stdout。
    pub fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "bonfire {args:?} 失败：\nstdout：{}\nstderr：{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    pub fn json(&self, args: &[&str]) -> serde_json::Value {
        let mut a = args.to_vec();
        a.push("--json");
        serde_json::from_str(self.ok(&a).trim()).expect("输出不是 JSON")
    }
}

pub fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let dest = to.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CliCases {
    pub case: Vec<CliCase>,
}

#[derive(Debug, Deserialize)]
pub struct CliCase {
    pub id: String,
    pub title: BTreeMap<String, String>,
    /// 复制 tests/inputs/saves/<fixture> 为存档目录 {saves}。
    #[serde(default)]
    pub fixture: Option<String>,
    /// 复制 tests/inputs/roots/<root> 到沙箱根目录（伪造的平台安装与用户规则）。
    #[serde(default)]
    pub root: Option<String>,
    /// 先依次执行的命令（必须成功）。
    #[serde(default)]
    pub pre: Vec<Vec<String>>,
    /// 执行前写入存档目录的文件（相对路径 → 内容）。
    #[serde(default)]
    pub edit: BTreeMap<String, String>,
    /// 执行前删除的文件或目录（支持占位符）。
    #[serde(default)]
    pub remove: Vec<String>,
    pub args: Vec<String>,
    pub expect_exit: i32,
    #[serde(default)]
    pub stdout_contains: Vec<String>,
    #[serde(default)]
    pub stdout_not_contains: Vec<String>,
    #[serde(default)]
    pub stderr_contains: Vec<String>,
    #[serde(default)]
    pub expect_stdout_json: BTreeMap<String, toml::Value>,
    #[serde(default)]
    pub expect_stderr_json: BTreeMap<String, toml::Value>,
    /// 执行后存档目录中文件的内容（相对路径 → 内容）。
    #[serde(default)]
    pub expect_files: BTreeMap<String, String>,
    #[serde(default)]
    pub in_docs: bool,
}

impl CliCase {
    /// 文档中展示的命令行（占位符换成示意路径）。
    pub fn doc_command(&self) -> String {
        let args: Vec<String> = self
            .args
            .iter()
            .map(|a| {
                let a = a
                    .replace("{inputs}", "tests/inputs")
                    .replace("{saves}", "D:/Saves/HollowKnight")
                    .replace("{tmp}", "D:/Temp");
                if a.contains(' ') || a.is_empty() {
                    format!("\"{a}\"")
                } else {
                    a
                }
            })
            .collect();
        format!("bonfire {}", args.join(" "))
    }
}

pub fn load_cli_cases() -> Vec<CliCase> {
    let p = root().join("tests/cases/cli_cases.toml");
    let text = std::fs::read_to_string(&p).expect("读取 cli_cases.toml");
    toml::from_str::<CliCases>(&text)
        .expect("cli_cases.toml 格式错误")
        .case
}

/// 按 `a.b.0.c` 字段路径检查 JSON 输出（数字段为数组下标；取最后一个非空行解析）。
pub fn check_json(
    stream: &str,
    text: &str,
    expect: &BTreeMap<String, toml::Value>,
) -> Result<(), String> {
    if expect.is_empty() {
        return Ok(());
    }
    let line = text
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("");
    let v: serde_json::Value =
        serde_json::from_str(line).map_err(|e| format!("{stream} 不是 JSON（{e}）"))?;
    for (path, want) in expect {
        let got = path
            .split('.')
            .try_fold(&v, |cur, k| match k.parse::<usize>() {
                Ok(i) if cur.is_array() => cur.get(i),
                _ => cur.get(k),
            });
        let want_json = serde_json::to_value(want).unwrap();
        if got != Some(&want_json) {
            return Err(format!(
                "{stream} 的 {path} 应为 {want_json}，实际为 {got:?}"
            ));
        }
    }
    Ok(())
}

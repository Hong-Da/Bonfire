//! 文档测试：保证文档与代码一致。
//!
//! 生成类内容（config.example.toml、docs/cli.md 示例）与重新生成的结果不一致时失败；
//! 设置环境变量 `BONFIRE_BLESS=1` 运行则直接改写为最新结果（test.bat --bless）。

#[path = "../common.rs"]
mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use common::root;

const IGNORED_DIRS: &[&str] = &["target", "dist", "node_modules", "data", "logs", "output", "history", "gen"];

fn bless() -> bool {
    std::env::var("BONFIRE_BLESS").is_ok_and(|v| v == "1")
}

/// 比较或改写生成内容。
fn check_generated(path: &Path, expected: &str) {
    let actual = std::fs::read_to_string(path).unwrap_or_default().replace("\r\n", "\n");
    if actual == expected {
        return;
    }
    if bless() {
        std::fs::write(path, expected).unwrap();
        return;
    }
    panic!("{} 不是最新，请运行 test.bat --bless 重新生成", path.display());
}

// ---------- 配置示例 ----------

#[test]
fn config_example_is_up_to_date() {
    check_generated(&root().join("config/config.example.toml"), &bonfire_app::Config::example_toml());
}

// ---------- CLI 文档示例 ----------

const BEGIN: &str = "<!-- examples:begin";
const END: &str = "<!-- examples:end -->";

fn render_cli_examples() -> String {
    let mut s = String::new();
    for c in common::load_cli_cases().iter().filter(|c| c.in_docs) {
        let title = c.title.get("zh-CN").cloned().unwrap_or_else(|| c.id.clone());
        s.push_str(&format!("### {title}\n\n```bat\n{}\n```\n\n退出码：{}\n\n", c.doc_command(), c.expect_exit));
    }
    s
}

#[test]
fn cli_doc_examples_are_up_to_date() {
    let path = root().join("docs/cli.md");
    let doc = std::fs::read_to_string(&path).expect("读取 docs/cli.md").replace("\r\n", "\n");
    let b = doc.find(BEGIN).expect("docs/cli.md 缺少 examples:begin 标记");
    let b_end = b + doc[b..].find("-->").unwrap() + 3;
    let e = doc.find(END).expect("docs/cli.md 缺少 examples:end 标记");
    let expected = format!("{}\n\n{}{}", &doc[..b_end], render_cli_examples(), &doc[e..]);
    check_generated(&path, &expected);
}

// ---------- 目录结构 ----------

/// 解析 README「项目结构」中的目录树，返回 (路径, 是否目录)。
fn readme_tree() -> Vec<(PathBuf, bool)> {
    let readme = std::fs::read_to_string(root().join("README.md")).unwrap();
    let section = &readme[readme.find("## 项目结构").expect("README 缺少「项目结构」")..];
    let block = section.split("```").nth(1).expect("「项目结构」中缺少目录树代码块");
    let mut stack: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for line in block.lines() {
        // 每层缩进是 4 个字符（"│   " 或 "    "），后跟 "├── " / "└── "。
        let Some(pos) = line.find("── ") else { continue };
        let depth = line[..pos].chars().count() / 4;
        let rest: String = line.chars().collect::<Vec<_>>()[line[..pos].chars().count() + 3..].iter().collect();
        let name = rest.split("  ").next().unwrap().trim().to_string();
        stack.truncate(depth);
        let is_dir = name.ends_with('/');
        for part in name.split(" / ") {
            let part = part.trim().trim_end_matches('/');
            let mut p: PathBuf = stack.iter().collect();
            p.push(part);
            out.push((p, is_dir));
        }
        if is_dir {
            stack.push(name.trim_end_matches('/').to_string());
        }
    }
    out
}

#[test]
fn readme_structure_matches_disk() {
    let tree = readme_tree();
    assert!(!tree.is_empty());
    let mut missing = Vec::new();
    for (p, is_dir) in &tree {
        if p.iter().any(|c| IGNORED_DIRS.contains(&c.to_str().unwrap())) {
            continue;
        }
        let full = root().join(p);
        let ok = if *is_dir { full.is_dir() } else { full.exists() };
        if !ok {
            missing.push(p.display().to_string());
        }
    }
    assert!(missing.is_empty(), "README「项目结构」中列出但不存在：{missing:?}");

    // 列出了子目录的目录，其实际子目录也都必须列出。
    let listed: BTreeSet<PathBuf> = tree.iter().filter(|(_, d)| *d).map(|(p, _)| p.clone()).collect();
    let mut parents: BTreeSet<PathBuf> = listed.iter().filter_map(|p| p.parent().map(Path::to_path_buf)).collect();
    parents.insert(PathBuf::new());
    let mut unlisted = Vec::new();
    for parent in parents {
        for entry in std::fs::read_dir(root().join(&parent)).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !entry.path().is_dir() || name.starts_with('.') || IGNORED_DIRS.contains(&name.as_str()) {
                continue;
            }
            let rel = parent.join(&name);
            if !listed.contains(&rel) {
                unlisted.push(rel.display().to_string());
            }
        }
    }
    assert!(unlisted.is_empty(), "存在但 README「项目结构」中没有列出：{unlisted:?}");
}

// ---------- 链接 ----------

fn markdown_files() -> Vec<PathBuf> {
    let r = root();
    let mut files = vec![r.join("README.md"), r.join("CLAUDE.md"), r.join("tests/README.md"), r.join("tests/inputs/README.md")];
    for e in std::fs::read_dir(r.join("docs")).unwrap().flatten() {
        if e.path().extension().is_some_and(|x| x == "md") {
            files.push(e.path());
        }
    }
    files
}

#[test]
fn relative_links_exist() {
    let mut broken = Vec::new();
    for f in markdown_files() {
        let text = std::fs::read_to_string(&f).unwrap();
        for (i, _) in text.match_indices("](") {
            let rest = &text[i + 2..];
            let Some(end) = rest.find(')') else { continue };
            let target = rest[..end].split('#').next().unwrap().trim();
            if target.is_empty() || target.contains("://") || target.starts_with("mailto:") {
                continue;
            }
            if !f.parent().unwrap().join(target).exists() {
                broken.push(format!("{} → {target}", f.strip_prefix(root()).unwrap().display()));
            }
        }
    }
    assert!(broken.is_empty(), "无效的相对链接：{broken:?}");
}

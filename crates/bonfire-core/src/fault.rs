//! 故障注入：用于崩溃一致性测试。
//!
//! 设置环境变量 `BONFIRE_FAULT_INJECT=<点名>[:<第 N 次>]` 后，进程在第 N 次（默认第 1 次）
//! 经过该点时立即中止（`process::abort`，不执行析构、不刷新缓冲），模拟断电或强杀。
//! 未设置时开销只有一次环境变量读取。

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

pub const ENV: &str = "BONFIRE_FAULT_INJECT";

/// 写入流程中的注入点（按发生顺序）。
pub mod points {
    /// 临时文件已写入、尚未重命名。
    pub const BEFORE_RENAME: &str = "before-rename";
    pub const AFTER_CHUNK: &str = "after-chunk";
    pub const AFTER_BLOB: &str = "after-blob";
    pub const AFTER_TREE: &str = "after-tree";
    pub const AFTER_SNAPSHOT: &str = "after-snapshot";
}

struct Plan {
    point: String,
    nth: u64,
    hits: AtomicU64,
}

fn plan() -> Option<&'static Plan> {
    static PLAN: OnceLock<Option<Plan>> = OnceLock::new();
    PLAN.get_or_init(|| {
        let v = std::env::var(ENV).ok().filter(|v| !v.is_empty())?;
        let (point, nth) = match v.split_once(':') {
            Some((p, n)) => (p.to_string(), n.parse().unwrap_or(1)),
            None => (v, 1),
        };
        Some(Plan { point, nth: nth.max(1), hits: AtomicU64::new(0) })
    })
    .as_ref()
}

/// 经过注入点。命中时中止进程。
pub fn point(name: &str) {
    if let Some(p) = plan()
        && p.point == name
        && p.hits.fetch_add(1, Ordering::SeqCst) + 1 == p.nth
    {
        eprintln!("[fault-inject] 在 {name} 处中止");
        std::process::abort();
    }
}

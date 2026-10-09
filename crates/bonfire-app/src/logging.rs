//! 日志：写入 logs 目录（UTF-8，按日期轮转，保留 N 个文件），可选同时输出到 stderr。
//!
//! CLI 的 stdout 只放结果，所以控制台日志一律走 stderr。

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::{EnvFilter, Layer, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::Log;

/// 初始化全局日志。返回的 guard 需要保存到程序结束，否则缓冲的日志会丢失。
/// 日志目录无法创建时只输出到 stderr（如开启），不阻止程序运行。
pub fn init(logs_dir: &Path, log: &Log, stderr: bool) -> Option<WorkerGuard> {
    let level = log.level.to_ascii_lowercase();
    let file = std::fs::create_dir_all(logs_dir).ok().and_then(|_| {
        Builder::new()
            .rotation(Rotation::DAILY)
            .filename_prefix("bonfire")
            .filename_suffix("log")
            .max_log_files(log.keep_days as usize)
            .build(logs_dir)
            .ok()
    });
    let (file_layer, guard) = match file {
        Some(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let layer = fmt::layer()
                .with_writer(writer)
                .with_ansi(false)
                .with_filter(EnvFilter::new(&level));
            (Some(layer), Some(guard))
        }
        None => (None, None),
    };
    let stderr_layer = stderr.then(|| {
        fmt::layer()
            .with_writer(std::io::stderr)
            .with_filter(EnvFilter::new(&level))
    });
    // 重复初始化（例如测试中）时忽略错误。
    let _ = tracing_subscriber::registry()
        .with(file_layer)
        .with(stderr_layer)
        .try_init();
    guard
}

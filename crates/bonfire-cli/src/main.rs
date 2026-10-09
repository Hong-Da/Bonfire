//! 命令行 `bonfire`：只做参数解析和输出格式化，业务逻辑在 bonfire-app。

mod cli;
mod commands;
mod exit;

use std::path::PathBuf;
use std::process::ExitCode;

use bonfire_app::{App, AppPaths, Config, ErrorKind, Locale, t};
use clap::FromArgMatches;

use cli::{Cli, OutputFormat};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    let mut paths = AppPaths::resolve();
    if let Some(c) = arg_value(&args, "--config") {
        paths.config_file = PathBuf::from(c);
    }
    let config = Config::load(&paths.config_file);
    let early = cli::early_locale(&args, || config.as_ref().ok().and_then(|c| c.general.language.clone()));

    // 帮助与参数错误按语言输出；clap 在 --help / --version 时也走错误分支。
    let parsed = cli::command(early).try_get_matches_from(&args).and_then(|m| Cli::from_arg_matches(&m));
    let cli = match parsed {
        Ok(c) => c,
        Err(e) => {
            let _ = e.print();
            return ExitCode::from(if e.use_stderr() { exit::USAGE } else { exit::OK });
        }
    };
    let format = cli.global.format();

    let config = match config {
        Ok(c) => c,
        Err(e) => return fail(format, early, exit::CONFIG, "config", "cli.error.config", &e.to_string()),
    };
    let locale = Locale::resolve(cli.global.lang.as_deref().or(config.general.language.as_deref()));
    let _log_guard = bonfire_app::logging::init(&paths.logs_dir, &config.log, cli.global.verbose);
    let repo_dir = cli.global.repo.clone().unwrap_or_else(|| config.repo_dir(&paths.default_repo_dir));
    tracing::debug!(?paths, repo = %repo_dir.display(), "bonfire 启动");

    let out = commands::Out { format, locale, quiet: cli.global.quiet, yes: cli.global.yes };
    let result = App::open(paths, config, locale, &repo_dir).and_then(|mut app| commands::run(&mut app, cli.cmd, &out));
    match result {
        Ok(()) => ExitCode::from(exit::OK),
        Err(e) => {
            tracing::warn!("命令失败：{e}");
            let (code, kind) = exit_code(e.kind());
            fail(format, locale, code, kind, "cli.error.general", &e.to_string())
        }
    }
}

/// 解析前读取某个参数的值（`--x v` 或 `--x=v`）。
fn arg_value(args: &[String], name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    args.iter().enumerate().find_map(|(i, a)| a.strip_prefix(&prefix).map(String::from).or_else(|| (a == name).then(|| args.get(i + 1).cloned()).flatten()))
}

fn exit_code(kind: ErrorKind) -> (u8, &'static str) {
    match kind {
        ErrorKind::General => (exit::GENERAL, "general"),
        ErrorKind::Usage => (exit::USAGE, "usage"),
        ErrorKind::Config => (exit::CONFIG, "config"),
        ErrorKind::NotFound => (exit::NOT_FOUND, "not-found"),
        ErrorKind::External => (exit::EXTERNAL, "external"),
        ErrorKind::Locked => (exit::REPO_LOCKED, "locked"),
        ErrorKind::GameRunning => (exit::GAME_RUNNING, "game-running"),
    }
}

/// 输出错误到 stderr 并返回退出码。JSON 模式下输出结构化错误。
fn fail(format: OutputFormat, locale: Locale, code: u8, kind: &str, title_key: &str, message: &str) -> ExitCode {
    match format {
        OutputFormat::Json => {
            let v = serde_json::json!({ "error": { "code": code, "kind": kind, "message": message } });
            eprintln!("{v}");
        }
        OutputFormat::Text => eprintln!("{}: {message}", t(locale, title_key)),
    }
    ExitCode::from(code)
}

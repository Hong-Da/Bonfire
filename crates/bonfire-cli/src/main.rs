//! 命令行 `bonfire`：只做参数解析和输出格式化，业务逻辑在 bonfire-app。

mod exit;

use std::path::PathBuf;
use std::process::ExitCode;

use bonfire_app::{AppPaths, Config, Locale, t};
use clap::{CommandFactory, Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Parser)]
#[command(name = "bonfire", version, about = "篝火 Bonfire：游戏存档备份与版本历史 / game save backup with version history")]
struct Cli {
    /// 存档仓库目录（覆盖配置）
    #[arg(long, global = true, value_name = "DIR")]
    repo: Option<PathBuf>,
    /// 配置文件路径
    #[arg(long, global = true, value_name = "FILE")]
    config: Option<PathBuf>,
    /// 输出语言：zh-CN / en
    #[arg(long, global = true, value_name = "LANG")]
    lang: Option<String>,
    /// 输出格式
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Text)]
    output: OutputFormat,
    /// 等同于 --output json
    #[arg(long, global = true)]
    json: bool,
    /// 减少输出
    #[arg(long, short, global = true)]
    quiet: bool,
    /// 在 stderr 输出日志
    #[arg(long, short, global = true)]
    verbose: bool,
    /// 跳过所有确认提示
    #[arg(long, short = 'y', global = true)]
    yes: bool,
}

impl Cli {
    fn format(&self) -> OutputFormat {
        if self.json { OutputFormat::Json } else { self.output }
    }
}

/// 应用运行上下文。
#[allow(dead_code)]
struct Ctx {
    paths: AppPaths,
    config: Config,
    locale: Locale,
    repo_dir: PathBuf,
    format: OutputFormat,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let format = cli.format();

    let mut paths = AppPaths::resolve();
    if let Some(c) = &cli.config {
        paths.config_file = c.clone();
    }
    let config = match Config::load(&paths.config_file) {
        Ok(c) => c,
        Err(e) => {
            // 配置都读不到时，语言只能来自参数或系统。
            let locale = Locale::resolve(cli.lang.as_deref());
            return fail(format, locale, exit::CONFIG, "cli.error.config", &e.to_string());
        }
    };
    let locale = Locale::resolve(cli.lang.as_deref().or(config.general.language.as_deref()));
    let _log_guard = bonfire_app::logging::init(&paths.logs_dir, &config.log, cli.verbose);
    let repo_dir = cli.repo.clone().unwrap_or_else(|| config.repo_dir(&paths.default_repo_dir));
    tracing::debug!(?paths, repo = %repo_dir.display(), "bonfire 启动");

    let _ctx = Ctx { paths, config, locale, repo_dir, format };

    // 子命令在 M1-5 起加入；目前没有子命令时显示帮助。
    if !cli.quiet {
        let _ = Cli::command().print_help();
    }
    ExitCode::from(exit::OK)
}

/// 输出错误到 stderr 并返回退出码。JSON 模式下输出结构化错误。
fn fail(format: OutputFormat, locale: Locale, code: u8, title_key: &str, message: &str) -> ExitCode {
    match format {
        OutputFormat::Json => {
            let v = serde_json::json!({ "error": { "code": code, "message": message } });
            eprintln!("{v}");
        }
        OutputFormat::Text => eprintln!("{}: {message}", t(locale, title_key)),
    }
    ExitCode::from(code)
}

//! 命令行定义。说明文字不写在代码里，而是按语言从文案文件填入（见 [`localize`]）。

use std::path::PathBuf;

use bonfire_app::{Locale, i18n::lookup};
use clap::{Args, Command, CommandFactory, Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Parser)]
#[command(name = "bonfire", version, disable_help_subcommand = true)]
pub struct Cli {
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Debug, Args)]
pub struct Global {
    #[arg(long, global = true, value_name = "DIR")]
    pub repo: Option<PathBuf>,
    #[arg(long, global = true, value_name = "FILE")]
    pub config: Option<PathBuf>,
    #[arg(long, global = true, value_name = "LANG")]
    pub lang: Option<String>,
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
    #[arg(long, global = true)]
    pub json: bool,
    #[arg(long, short, global = true)]
    pub quiet: bool,
    #[arg(long, short, global = true)]
    pub verbose: bool,
    #[arg(long, short = 'y', global = true)]
    pub yes: bool,
}

impl Global {
    pub fn format(&self) -> OutputFormat {
        if self.json {
            OutputFormat::Json
        } else {
            self.output
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Cmd {
    Game {
        #[command(subcommand)]
        cmd: GameCmd,
    },
    Scan {
        #[arg(long, value_name = "PLATFORM")]
        platform: Option<String>,
        #[arg(long)]
        no_discover: bool,
        #[arg(long)]
        no_metadata: bool,
        #[arg(long)]
        force_verify: bool,
    },
    Rules {
        #[command(subcommand)]
        cmd: RulesCmd,
    },
    Info {
        game: String,
        #[arg(long)]
        refresh: bool,
        #[arg(long = "set", value_name = "FIELD=VALUE")]
        set: Vec<String>,
        #[arg(long, value_name = "FIELD")]
        unlock: Vec<String>,
        #[arg(long)]
        search: bool,
        #[arg(long, value_name = "SOURCE:ID")]
        pick: Option<String>,
    },
    Status {
        game: Option<String>,
        #[arg(long)]
        force_verify: bool,
    },
    Backup {
        games: Vec<String>,
        #[arg(short = 'm', long)]
        message: Option<String>,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        force: bool,
        #[arg(long, conflicts_with_all = ["games", "profile"])]
        all: bool,
        #[arg(long, conflicts_with_all = ["games", "profile", "all", "message", "tag", "force"])]
        auto: bool,
        #[arg(long, value_name = "NAME", conflicts_with = "move_branch")]
        new_branch: Option<String>,
        #[arg(long)]
        move_branch: bool,
    },
    Log {
        game: String,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        grep: Option<String>,
        #[arg(long, default_value_t = bonfire_app::history::DEFAULT_LOG_LIMIT)]
        limit: usize,
    },
    Diff {
        game: String,
        a: String,
        b: Option<String>,
    },
    Restore {
        game: String,
        target: String,
        #[arg(long = "path", value_name = "GLOB")]
        paths: Vec<String>,
        #[arg(long)]
        full: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        force: bool,
    },
    UndoRestore {
        game: String,
    },
    Comment {
        game: String,
        snapshot: String,
        #[arg(short = 'm', long, required = true)]
        message: String,
    },
    Branch {
        game: String,
        name: Option<String>,
        snapshot: Option<String>,
        #[arg(long, short = 'd', value_name = "NAME", conflicts_with_all = ["name", "snapshot", "rename"])]
        delete: Option<String>,
        #[arg(long, num_args = 2, value_names = ["OLD", "NEW"], conflicts_with_all = ["name", "snapshot"])]
        rename: Option<Vec<String>>,
        #[arg(long, num_args = 0..=1, value_name = "NAME", conflicts_with_all = ["name", "snapshot", "delete", "rename"])]
        primary: Option<Option<String>>,
        #[arg(long, requires = "primary")]
        tidy: bool,
        #[arg(long, requires = "delete")]
        dry_run: bool,
    },
    Tag {
        game: String,
        name: Option<String>,
        snapshot: Option<String>,
        #[arg(long, short = 'd', value_name = "NAME", conflicts_with_all = ["name", "snapshot"])]
        delete: Option<String>,
    },
    Rm {
        game: String,
        #[arg(required = true)]
        snapshots: Vec<String>,
        #[arg(long)]
        dry_run: bool,
    },
    Profile {
        #[command(subcommand)]
        cmd: ProfileCmd,
    },
    Export {
        game: String,
        snapshot: Option<String>,
        #[arg(
            long,
            value_name = "FILE",
            required_unless_present = "folder",
            conflicts_with = "folder"
        )]
        zip: Option<PathBuf>,
        #[arg(long, value_name = "DIR")]
        folder: Option<PathBuf>,
    },
    Import {
        game: String,
        source: PathBuf,
        #[arg(long, value_name = "NAME", conflicts_with = "onto")]
        branch: Option<String>,
        #[arg(long, value_name = "SNAPSHOT", conflicts_with = "onto")]
        parent: Option<String>,
        #[arg(long, value_name = "BRANCH")]
        onto: Option<String>,
        #[arg(short = 'm', long)]
        message: Option<String>,
        #[arg(long, conflicts_with_all = ["parent", "onto"])]
        download: bool,
        #[arg(long, value_name = "PASSWORD", requires = "download")]
        password: Option<String>,
        #[arg(long, requires = "download")]
        replace: bool,
        #[arg(long)]
        dry_run: bool,
    },
    Bundle {
        #[command(subcommand)]
        cmd: BundleCmd,
    },
    Shots {
        #[command(subcommand)]
        cmd: ShotsCmd,
    },
    Fsck {
        #[arg(long)]
        deep: bool,
    },
    Gc {
        games: Vec<String>,
        #[arg(long)]
        objects_only: bool,
        #[arg(long)]
        dry_run: bool,
    },
    Reindex,
    Activity {
        #[arg(long, value_enum)]
        category: Option<ActivityCategoryArg>,
        #[arg(long)]
        failed: bool,
        #[arg(long)]
        game: Option<String>,
        #[arg(long)]
        grep: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    Stats,
    Repo {
        #[command(subcommand)]
        cmd: RepoCmd,
    },
    Sync {
        #[command(subcommand)]
        cmd: SyncCmd,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ActivityCategoryArg {
    Backup,
    Restore,
    History,
    Sync,
    Shots,
    Transfer,
    Maintain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SyncKindArg {
    Github,
    Gitee,
    Webdav,
    Folder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum SyncContentArg {
    #[default]
    Repo,
    Latest,
    Shots,
}

/// 可以忽略的同步问题。
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SyncIgnoreArg {
    Network,
    Size,
    Excluded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum SyncScheduleArg {
    #[default]
    Manual,
    AfterBackup,
    Daily,
    Hours,
}

#[derive(Debug, Subcommand)]
pub enum SyncCmd {
    List,
    Add {
        name: String,
        #[arg(long, value_enum)]
        kind: SyncKindArg,
        #[arg(long)]
        url: String,
        #[arg(long, default_value = "main")]
        branch: String,
        #[arg(long, default_value = "")]
        user: String,
        #[arg(long, value_enum, default_value_t)]
        content: SyncContentArg,
        #[arg(long, conflicts_with = "no_shots")]
        shots: bool,
        #[arg(long)]
        no_shots: bool,
        #[arg(long)]
        no_settings: bool,
        #[arg(long, value_delimiter = ',')]
        games: Vec<String>,
        #[arg(long)]
        max_file_mb: Option<u64>,
        #[arg(long)]
        max_total_mb: Option<u64>,
        #[arg(long, value_enum, default_value_t)]
        schedule: SyncScheduleArg,
        #[arg(long, value_enum, value_delimiter = ',')]
        ignore: Vec<SyncIgnoreArg>,
    },
    Rm {
        target: String,
    },
    Test {
        target: String,
    },
    Preview {
        target: String,
    },
    Push {
        targets: Vec<String>,
    },
    Pull {
        target: String,
        dest: PathBuf,
    },
    Compact {
        target: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum RepoCmd {
    Status,
    Encrypt {
        #[arg(long)]
        no_remember: bool,
    },
    Unlock {
        #[arg(long)]
        remember: bool,
    },
    Passwd,
    Decrypt,
    Forget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ShotSwitch {
    On,
    Off,
    Auto,
}

#[derive(Debug, Subcommand)]
pub enum ShotsCmd {
    List {
        game: String,
    },
    Sync {
        games: Vec<String>,
    },
    Export {
        game: String,
        #[arg(short = 'o', long = "out", value_name = "PATH")]
        out_file: PathBuf,
        #[arg(long)]
        folder: bool,
        #[arg(long = "id", value_name = "ID")]
        ids: Vec<String>,
        #[arg(long, value_name = "DATE")]
        since: Option<String>,
        #[arg(long, value_name = "SNAPSHOT")]
        snapshot: Option<String>,
    },
    Rm {
        game: String,
        #[arg(required = true)]
        ids: Vec<String>,
        #[arg(long)]
        purge: bool,
        #[arg(long, requires = "purge")]
        delete_originals: bool,
    },
    Restore {
        game: String,
        #[arg(required = true)]
        ids: Vec<String>,
    },
    Trash {
        game: String,
        #[arg(long)]
        empty: bool,
        #[arg(long, requires = "empty")]
        delete_originals: bool,
    },
    Set {
        game: String,
        #[arg(value_enum)]
        value: ShotSwitch,
    },
    Found,
    Diff {
        game: String,
    },
    WriteBack {
        game: String,
        ids: Vec<String>,
        #[arg(long, value_name = "DIR")]
        to: Option<PathBuf>,
    },
    Recollect {
        game: String,
    },
    Compare {
        #[arg(long)]
        write_back: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OnOff {
    On,
    Off,
}

#[derive(Debug, Subcommand)]
pub enum ShotsDirCmd {
    Add { game: String, dir: PathBuf },
    Rm { game: String, dir: String },
}

#[derive(Debug, Subcommand)]
pub enum BundleCmd {
    Export {
        #[arg(required_unless_present = "all")]
        games: Vec<String>,
        #[arg(long, conflicts_with = "games")]
        all: bool,
        #[arg(short = 'o', long = "out", value_name = "FILE")]
        out_file: PathBuf,
        #[arg(long = "branch", value_name = "NAME", conflicts_with_all = ["from", "to"])]
        branches: Vec<String>,
        #[arg(long, value_name = "SNAPSHOT", requires = "to")]
        from: Option<String>,
        #[arg(long, value_name = "SNAPSHOT")]
        to: Option<String>,
        #[arg(long)]
        shots: bool,
        #[arg(long, requires = "shots", conflicts_with_all = ["branches", "from", "to"])]
        no_saves: bool,
        #[arg(long, conflicts_with_all = ["branches", "from", "to"])]
        zip: bool,
    },
    Import {
        file: PathBuf,
        #[arg(long, conflicts_with = "only")]
        game: Option<String>,
        #[arg(long, value_name = "GAME")]
        only: Vec<String>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long, conflicts_with = "no_shots")]
        no_saves: bool,
        #[arg(long)]
        no_shots: bool,
        #[arg(long, conflicts_with = "no_shots")]
        shots_to_disk: bool,
        #[arg(long, conflicts_with = "no_saves")]
        adopt_main: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum GameCmd {
    Add {
        name: String,
        #[arg(
            long = "path",
            value_name = "PATH",
            required_unless_present = "config_paths"
        )]
        paths: Vec<PathBuf>,
        #[arg(long = "config-path", value_name = "PATH")]
        config_paths: Vec<PathBuf>,
        #[arg(long, value_name = "DIR")]
        install_dir: Option<PathBuf>,
    },
    List {
        #[arg(long)]
        all: bool,
    },
    Rm {
        game: String,
        #[arg(long)]
        delete_snapshots: bool,
        #[arg(long)]
        delete_shots: bool,
    },
    Include {
        games: Vec<String>,
        #[arg(long, conflicts_with = "games")]
        all_new: bool,
        #[arg(long, value_enum, default_value = "both")]
        mode: ModeArg,
    },
    Ignore {
        #[arg(required = true)]
        games: Vec<String>,
    },
    Confirm {
        game: String,
    },
    ShotsDir {
        #[command(subcommand)]
        cmd: ShotsDirCmd,
    },
    Process {
        game: String,
        #[arg(long, value_name = "NAME")]
        add: Vec<String>,
        #[arg(long, value_name = "NAME")]
        rm: Vec<String>,
        #[arg(long, conflicts_with_all = ["add", "rm"])]
        clear: bool,
    },
    AddShots {
        name: String,
        #[arg(long, value_name = "APPID")]
        steam: Option<String>,
        #[arg(long = "dir", value_name = "DIR")]
        dirs: Vec<PathBuf>,
    },
    ShotsOnly {
        game: String,
        #[arg(value_enum)]
        value: OnOff,
    },
    Mode {
        game: String,
        #[arg(value_enum)]
        mode: Option<ModeArg>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ModeArg {
    Both,
    Saves,
    Shots,
}

#[derive(Debug, Subcommand)]
pub enum RulesCmd {
    Update {
        #[arg(long, value_name = "FILE")]
        bundle: Option<PathBuf>,
        #[arg(long, value_name = "URL")]
        url: Option<String>,
    },
    Info,
}

#[derive(Debug, Subcommand)]
pub enum ProfileCmd {
    List,
    Save {
        name: String,
        #[arg(required = true)]
        games: Vec<String>,
        #[arg(short = 'm', long)]
        message: Option<String>,
    },
    Rm {
        name: String,
    },
}

/// 构造带本地化说明的命令定义。
pub fn command(locale: Locale) -> Command {
    localize(Cli::command(), locale, "")
}

/// 按 `cli.cmd.<路径>` 与 `cli.arg.<路径>.<参数>` 填入说明；参数说明找不到时依次向上一级路径查找，最后查 `cli.arg.<参数>`。
fn localize(mut cmd: Command, l: Locale, path: &str) -> Command {
    let key = if path.is_empty() {
        "cli.cmd.root".to_string()
    } else {
        format!("cli.cmd.{path}")
    };
    if let Some(text) = lookup(l, &key) {
        cmd = cmd.about(text);
    }
    let ids: Vec<String> = cmd
        .get_arguments()
        .map(|a| a.get_id().to_string())
        .collect();
    for id in ids {
        if let Some(text) = arg_help(l, path, &id) {
            cmd = cmd.mut_arg(id.as_str(), |a| a.help(text));
        }
    }
    let subs: Vec<String> = cmd
        .get_subcommands()
        .map(|s| s.get_name().to_string())
        .collect();
    for name in subs {
        let sub_path = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}.{name}")
        };
        cmd = cmd.mut_subcommand(name.as_str(), |s| localize(s, l, &sub_path));
    }
    cmd
}

pub fn arg_help(l: Locale, path: &str, id: &str) -> Option<&'static str> {
    let mut p = path.to_string();
    loop {
        let key = if p.is_empty() {
            format!("cli.arg.{id}")
        } else {
            format!("cli.arg.{p}.{id}")
        };
        if let Some(t) = lookup(l, &key) {
            return Some(t);
        }
        // 分支与标签共用 `cli.arg.ref.*`。
        if (p == "branch" || p == "tag")
            && let Some(t) = lookup(l, &format!("cli.arg.ref.{id}"))
        {
            return Some(t);
        }
        if p.is_empty() {
            return None;
        }
        p = p
            .rsplit_once('.')
            .map(|(a, _)| a.to_string())
            .unwrap_or_default();
    }
}

/// 解析参数之前确定语言：`--lang` 参数 > 环境变量 > 配置文件 > 系统语言。
pub fn early_locale(args: &[String], config_lang: impl FnOnce() -> Option<String>) -> Locale {
    let from_args = args.iter().enumerate().find_map(|(i, a)| {
        a.strip_prefix("--lang=")
            .map(String::from)
            .or_else(|| (a == "--lang").then(|| args.get(i + 1).cloned()).flatten())
    });
    let configured = from_args
        .or_else(|| std::env::var(bonfire_app::config::ENV_LANG).ok())
        .or_else(config_lang);
    Locale::resolve(configured.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个命令和参数在两种语言下都有说明。
    #[test]
    fn every_command_and_arg_is_documented() {
        fn walk(cmd: &Command, path: &str, l: Locale, missing: &mut Vec<String>) {
            let key = if path.is_empty() {
                "cli.cmd.root".to_string()
            } else {
                format!("cli.cmd.{path}")
            };
            if lookup(l, &key).is_none() {
                missing.push(format!("{} {key}", l.code()));
            }
            for a in cmd.get_arguments() {
                if arg_help(l, path, a.get_id().as_str()).is_none() {
                    missing.push(format!("{} {path}:{}", l.code(), a.get_id()));
                }
            }
            for s in cmd.get_subcommands() {
                let p = if path.is_empty() {
                    s.get_name().to_string()
                } else {
                    format!("{path}.{}", s.get_name())
                };
                walk(s, &p, l, missing);
            }
        }
        let mut missing = Vec::new();
        for l in Locale::ALL {
            let mut cmd = Cli::command();
            cmd.build();
            walk(&cmd, "", l, &mut missing);
        }
        assert!(missing.is_empty(), "缺少说明：{missing:#?}");
    }

    #[test]
    fn help_follows_locale() {
        let zh = command(Locale::ZhCn).render_long_help().to_string();
        let en = command(Locale::En).render_long_help().to_string();
        assert!(zh.contains("备份游戏"), "{zh}");
        assert!(en.contains("Back up games"), "{en}");
    }

    #[test]
    fn early_locale_from_args() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            early_locale(&args(&["bonfire", "--lang", "en", "status"]), || None),
            Locale::En
        );
        assert_eq!(
            early_locale(&args(&["bonfire", "--lang=zh-CN"]), || Some("en".into())),
            Locale::ZhCn
        );
    }

    #[test]
    fn definition_is_valid() {
        Cli::command().debug_assert();
    }
}

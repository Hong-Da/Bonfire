//! 各子命令：调用 bonfire-app，按格式输出。不含业务逻辑。

use std::io::{BufRead, IsTerminal, Write};

use bonfire_app::app::RemoveOptions;
use bonfire_app::backup::{BackupOptions, BackupResult, DetachedChoice, GameStatus};
use bonfire_app::core::index::{GameRecord, Profile};
use bonfire_app::core::refs::RefKind;
use bonfire_app::core::scan::FileStatus;
use bonfire_app::history::DiffStatus;
use bonfire_app::restore::{Action, RestoreOptions, RestoreReport};
use bonfire_app::shots::ManageMode;
use bonfire_app::transfer::{
    ArchivePick, BundleImportReport, BundleScope, ExportFormat, ImportFilesOptions, ImportPlace,
    PackageOptions, RefAction,
};
use bonfire_app::util::human_bytes;
use bonfire_app::{App, AppError, GameState, Locale, NewGame, tf};
use serde_json::{Value, json};

use std::path::{Path, PathBuf};

use crate::cli::{
    BundleCmd, Cmd, GameCmd, ModeArg, OnOff, OutputFormat, ProfileCmd, RulesCmd, ShotSwitch,
    ShotsCmd, ShotsDirCmd,
};

type Result<T> = std::result::Result<T, AppError>;

pub struct Out {
    pub format: OutputFormat,
    pub locale: Locale,
    pub quiet: bool,
    pub yes: bool,
}

impl Out {
    fn json(&self) -> bool {
        self.format == OutputFormat::Json
    }

    /// 文本模式下输出一行；`--quiet` 时不输出。
    fn line(&self, s: impl AsRef<str>) {
        if !self.json() && !self.quiet {
            println!("{}", s.as_ref());
        }
    }

    fn emit(&self, v: Value) {
        if self.json() {
            println!("{v}");
        }
    }

    fn t(&self, key: &str) -> &'static str {
        // 文案表是 'static；键都已在文案文件中定义（由 CLI 测试覆盖）。
        bonfire_app::i18n::lookup(self.locale, key).unwrap_or("")
    }

    fn tf(&self, key: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
        tf(self.locale, key, args)
    }

    /// 危险操作的确认。`--yes` 直接通过；非交互环境没有 `--yes` 时报错。
    fn confirm(&self, prompt: String) -> Result<bool> {
        if self.yes {
            return Ok(true);
        }
        if !std::io::stdin().is_terminal() {
            return Err(AppError::Invalid(self.t("cli.error.need_yes").to_string()));
        }
        eprint!("{prompt}");
        let _ = std::io::stderr().flush();
        let mut s = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut s)
            .map_err(|e| AppError::io("stdin", e))?;
        Ok(matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "y" | "yes" | "是"
        ))
    }
}

/// 执行子命令。返回值为部分失败时的错误（例如批量备份中个别游戏失败）。
pub fn run(app: &mut App, cmd: Cmd, out: &Out) -> Result<()> {
    match cmd {
        Cmd::Game { cmd } => game(app, cmd, out),
        Cmd::Scan {
            platform,
            no_discover,
            no_metadata,
            force_verify,
        } => {
            app.fetch_metadata_on_scan = !no_metadata;
            let discovery = if no_discover {
                None
            } else {
                let lock = app.repo.lock()?;
                let report = app.scan_platforms(&lock, platform.as_deref())?;
                print_discovery(&report, out);
                Some(report)
            };
            let rows = status_table(app, force_verify, out)?;
            out.emit(json!({ "discovery": discovery, "games": rows }));
            Ok(())
        }
        Cmd::Rules { cmd } => rules(app, cmd, out),
        Cmd::Info {
            game,
            refresh,
            set,
            unlock,
            search,
            pick,
        } => info(app, &game, refresh, set, unlock, search, pick, out),
        Cmd::Status { game, force_verify } => match game {
            Some(key) => status_one(app, &key, force_verify, out),
            None => {
                let rows = status_table(app, force_verify, out)?;
                out.emit(Value::Array(rows));
                Ok(())
            }
        },
        Cmd::Backup {
            games,
            message,
            profile,
            tag,
            force,
            all,
            auto,
            new_branch,
            move_branch,
        } => {
            if auto {
                return auto_backup(app, out);
            }
            let on_detached = match (new_branch, move_branch) {
                (Some(n), _) => DetachedChoice::NamedBranch(n),
                (None, true) => DetachedChoice::MoveCurrent,
                _ => DetachedChoice::NewBranch,
            };
            backup(
                app,
                games,
                message,
                profile,
                tag,
                force,
                all,
                on_detached,
                out,
            )
        }
        Cmd::Log {
            game,
            branch,
            grep,
            limit,
        } => log(app, &game, branch.as_deref(), grep.as_deref(), limit, out),
        Cmd::Diff { game, a, b } => diff(app, &game, &a, b.as_deref(), out),
        Cmd::Restore {
            game,
            target,
            paths,
            full,
            dry_run,
            force,
        } => restore(
            app,
            &game,
            &target,
            RestoreOptions {
                paths,
                full,
                dry_run,
                force,
            },
            out,
        ),
        Cmd::UndoRestore { game } => undo(app, &game, out),
        Cmd::Comment {
            game,
            snapshot,
            message,
        } => {
            let game = app.find_game(&game)?;
            let lock = app.repo.lock()?;
            let id = app.set_comment(&lock, &game, &snapshot, &message)?;
            out.emit(json!({ "game": game.id, "snapshot": id, "comment": message.trim() }));
            out.line(out.tf("cli.out.comment_set", &[("id", &id.short(12))]));
            Ok(())
        }
        Cmd::Branch {
            game,
            delete: Some(name),
            dry_run,
            ..
        } => delete_branch(app, &game, &name, dry_run, out),
        Cmd::Branch {
            game,
            primary: Some(branch),
            tidy,
            ..
        } => {
            let g = app.find_game(&game)?;
            match branch {
                Some(b) => {
                    let lock = app.repo.lock()?;
                    let r = app.set_primary(&lock, &g, &b, tidy)?;
                    out.emit(serde_json::to_value(&r).unwrap());
                    for (from, to) in &r.renamed {
                        out.line(out.tf("cli.out.branch_renamed", &[("from", from), ("to", to)]));
                    }
                    out.line(out.tf("cli.out.primary_set", &[("branch", &r.branch)]));
                }
                None => {
                    let views = app.primaries(&g.id)?;
                    out.emit(json!({ "primary": views[0].branch, "offPrimary": app.off_primary(&g.id)?, "machines": views }));
                    for v in &views {
                        let who = if v.mine {
                            out.t("cli.out.primary_mine").to_string()
                        } else if v.label.is_empty() {
                            v.key.clone()
                        } else {
                            v.label.clone()
                        };
                        out.line(format!("{who}: heads/{}", v.branch));
                    }
                    if app.off_primary(&g.id)? {
                        out.line(out.t("cli.out.primary_off"));
                    }
                }
            }
            Ok(())
        }
        Cmd::Branch {
            game,
            rename: Some(pair),
            ..
        } => {
            let g = app.find_game(&game)?;
            let lock = app.repo.lock()?;
            app.rename_branch(&lock, &g, &pair[0], &pair[1])?;
            out.emit(json!({ "renamed": { "from": pair[0], "to": pair[1] } }));
            out.line(out.tf(
                "cli.out.branch_renamed",
                &[("from", &pair[0]), ("to", &pair[1])],
            ));
            Ok(())
        }
        Cmd::Branch {
            game,
            name,
            snapshot,
            ..
        } => refs(app, &game, RefKind::Branch, name, snapshot, out),
        Cmd::Tag {
            game,
            delete: Some(name),
            ..
        } => {
            let g = app.find_game(&game)?;
            let lock = app.repo.lock()?;
            app.delete_tag(&lock, &g, &name)?;
            out.emit(json!({ "deleted": format!("tags/{name}") }));
            out.line(out.tf("cli.out.tag_deleted", &[("name", &name)]));
            Ok(())
        }
        Cmd::Tag {
            game,
            name,
            snapshot,
            ..
        } => refs(app, &game, RefKind::Tag, name, snapshot, out),
        Cmd::Rm {
            game,
            snapshot,
            dry_run,
        } => delete_snapshot(app, &game, &snapshot, dry_run, out),
        Cmd::Profile { cmd } => profile(app, cmd, out),
        Cmd::Export {
            game,
            snapshot,
            zip,
            folder,
        } => export(app, &game, snapshot.as_deref(), zip, folder, out),
        Cmd::Import {
            game,
            source,
            branch,
            message,
            dry_run,
            download: true,
            password,
            replace,
            ..
        } => import_download(
            app, &game, &source, branch, message, password, replace, dry_run, out,
        ),
        Cmd::Import {
            game,
            source,
            branch,
            parent,
            onto,
            message,
            dry_run,
            ..
        } => {
            let place = match onto {
                Some(b) => ImportPlace::Continue { branch: b },
                None => ImportPlace::NewBranch {
                    name: branch,
                    parent,
                },
            };
            import(
                app,
                &game,
                &source,
                ImportFilesOptions {
                    place,
                    comment: message,
                    dry_run,
                },
                out,
            )
        }
        Cmd::Bundle { cmd } => bundle(app, cmd, out),
        Cmd::Shots { cmd } => shots(app, cmd, out),
        Cmd::Fsck { deep } => fsck(app, deep, out),
        Cmd::Reindex => {
            let lock = app.repo.lock()?;
            let s = app.reindex(&lock)?;
            out.emit(serde_json::to_value(&s).unwrap());
            out.line(out.tf(
                "cli.out.reindex_done",
                &[("snapshots", &s.snapshots), ("refs", &s.refs)],
            ));
            Ok(())
        }
    }
}

fn game_json(app: &App, g: &GameRecord) -> Result<Value> {
    let mut v = serde_json::to_value(g).unwrap();
    v["locations"] = serde_json::to_value(app.locations(&g.id)?).unwrap();
    Ok(v)
}

fn game(app: &mut App, cmd: GameCmd, out: &Out) -> Result<()> {
    match cmd {
        GameCmd::Add {
            name,
            paths,
            config_paths,
            install_dir,
        } => {
            let lock = app.repo.lock()?;
            let g = app.add_game(
                &lock,
                &NewGame {
                    name,
                    save_paths: paths,
                    config_paths,
                    install_dir,
                },
            )?;
            let v = game_json(app, &g)?;
            let n = v["locations"].as_array().map_or(0, Vec::len);
            out.emit(v);
            out.line(out.tf(
                "cli.out.game_added",
                &[("name", &g.name), ("id", &g.id), ("n", &n)],
            ));
        }
        GameCmd::Include {
            games,
            all_new,
            mode,
        } => {
            if games.is_empty() && !all_new {
                return Err(AppError::Invalid(
                    out.t("cli.error.include_what").to_string(),
                ));
            }
            let lock = app.repo.lock()?;
            let added = app.include_games(&lock, &games, all_new)?;
            // 按选择的管理方式加入（默认存档和截图）。
            if mode != ModeArg::Both {
                for g in &added {
                    app.set_manage_mode(g, mode_of(mode))?;
                }
            }
            // 加入后补全游戏信息（失败不影响）。
            for g in &added {
                let _ = app.refresh_metadata(g, false);
            }
            out.emit(json!({ "included": added.iter().map(|g| &g.id).collect::<Vec<_>>() }));
            if added.is_empty() {
                out.line(out.t("cli.out.nothing_new"));
            }
            for g in &added {
                out.line(out.tf("cli.out.game_included", &[("name", &g.name), ("id", &g.id)]));
            }
        }
        GameCmd::Ignore { games } => {
            let lock = app.repo.lock()?;
            let ignored = app.ignore_games(&lock, &games)?;
            out.emit(json!({ "ignored": ignored.iter().map(|g| &g.id).collect::<Vec<_>>() }));
            for g in &ignored {
                out.line(out.tf("cli.out.game_ignored", &[("name", &g.name)]));
            }
        }
        GameCmd::ShotsDir { cmd } => shots_dir(app, cmd, out)?,
        GameCmd::AddShots { name, steam, dirs } => {
            let lock = app.repo.lock()?;
            let key = steam.map(|id| format!("steam:{id}"));
            let g = app.add_shots_game(&lock, key.as_deref(), &name, &dirs)?;
            out.emit(game_json(app, &g)?);
            out.line(out.tf(
                "cli.out.added_shots_game",
                &[("name", &g.name), ("id", &g.id)],
            ));
        }
        GameCmd::ShotsOnly { game, value } => {
            let g = app.find_game(&game)?;
            let on = value == OnOff::On;
            app.set_shots_only(&g, on)?;
            out.emit(json!({ "game": g.id, "shotsOnly": on }));
            out.line(out.tf(
                if on {
                    "cli.out.shots_only_on"
                } else {
                    "cli.out.shots_only_off"
                },
                &[("game", &g.name)],
            ));
        }
        GameCmd::Mode { game, mode } => {
            let g = app.find_game(&game)?;
            if let Some(m) = mode {
                app.set_manage_mode(&g, mode_of(m))?;
            }
            let m = app.manage_mode(&g)?;
            out.emit(json!({ "game": g.id, "mode": m }));
            out.line(out.tf(
                if mode.is_some() {
                    "cli.out.mode_set"
                } else {
                    "cli.out.mode_is"
                },
                &[
                    ("game", &g.name),
                    ("mode", &out.t(&format!("cli.mode.{}", m.as_str()))),
                ],
            ));
        }
        GameCmd::Confirm { game } => {
            let lock = app.repo.lock()?;
            let (g, n) = app.confirm_locations(&lock, &game)?;
            out.emit(json!({ "game": g.id, "confirmed": n }));
            out.line(out.tf("cli.out.confirmed", &[("name", &g.name), ("n", &n)]));
        }
        GameCmd::List { all } => {
            let games = if all { app.all_games()? } else { app.games()? };
            out.emit(Value::Array(
                games
                    .iter()
                    .map(|g| game_json(app, g))
                    .collect::<Result<_>>()?,
            ));
            if games.is_empty() {
                out.line(out.t("cli.out.no_games"));
            }
            for g in &games {
                let n = app.locations(&g.id)?.len();
                let ignored = if g.enabled {
                    String::new()
                } else {
                    format!("  ({})", out.t("cli.out.ignored"))
                };
                out.line(format!(
                    "{:<20} {}  [{}]{ignored}",
                    g.id,
                    g.name,
                    out.tf("cli.out.locations", &[("n", &n)])
                ));
            }
        }
        GameCmd::Rm {
            game,
            delete_snapshots,
            delete_shots,
        } => {
            let g = app.find_game(&game)?;
            let mut extra = String::new();
            if delete_snapshots {
                extra.push_str(out.t("cli.confirm.rm_snapshots"));
            }
            if delete_shots {
                extra.push_str(out.t("cli.confirm.rm_shots"));
            }
            let text = out.tf("cli.confirm.rm", &[("game", &g.name), ("extra", &extra)]);
            if !out.confirm(text)? {
                out.line(out.t("cli.out.cancelled"));
                return Ok(());
            }
            let lock = app.repo.lock()?;
            let r = app.remove_game_with(
                &lock,
                &g.id,
                &RemoveOptions {
                    snapshots: delete_snapshots,
                    shots: delete_shots,
                },
            )?;
            let g = r.game;
            out.emit(json!({ "removed": g.id, "snapshots": r.snapshots, "shots": r.shots }));
            if delete_snapshots || delete_shots {
                out.line(out.tf(
                    "cli.out.rm_deleted",
                    &[("snapshots", &r.snapshots), ("shots", &r.shots)],
                ));
            }
            let key = if g.enabled {
                "cli.out.game_removed"
            } else {
                "cli.out.game_ignored"
            };
            out.line(out.tf(key, &[("name", &g.name)]));
        }
    }
    Ok(())
}

fn status_json(s: &GameStatus, with_changes: bool) -> Value {
    let mut v = json!({
        "id": s.game.id,
        "name": s.game.name,
        "state": s.state,
        "summary": s.summary,
        "files": s.files,
        "totalBytes": s.total_bytes,
        "head": s.head,
        "branch": s.branch,
        "detached": s.detached,
        "lastModifiedNs": s.last_modified_ns,
    });
    if with_changes {
        v["changes"] = serde_json::to_value(
            s.changes
                .iter()
                .filter(|c| c.status != FileStatus::Unchanged)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    }
    v
}

fn state_text(out: &Out, s: &GameStatus) -> String {
    let key = match s.state {
        GameState::NeverBackedUp => "cli.state.never-backed-up",
        GameState::HasChanges => "cli.state.has-changes",
        GameState::UpToDate => "cli.state.up-to-date",
        GameState::ShotsOnly => "cli.state.shots-only",
    };
    let mut text = out.t(key).to_string();
    if s.state == GameState::HasChanges {
        let sum = &s.summary;
        text += "  ";
        text += &out.tf(
            "cli.out.changes",
            &[
                ("added", &sum.added),
                ("modified", &sum.modified),
                ("deleted", &sum.deleted),
                ("bytes", &human_bytes(sum.changed_bytes)),
            ],
        );
    }
    text
}

fn game_status(app: &App, g: &GameRecord, force_verify: bool) -> Result<GameStatus> {
    if force_verify {
        app.status_verified(g)
    } else {
        app.status(g)
    }
}

/// 所有游戏的状态表（文本输出），返回 JSON 行。
fn status_table(app: &App, force_verify: bool, out: &Out) -> Result<Vec<Value>> {
    let games = app.games()?;
    let mut rows = Vec::new();
    if games.is_empty() {
        out.line(out.t("cli.out.no_games"));
    }
    for g in &games {
        let s = game_status(app, g, force_verify)?;
        out.line(format!(
            "{:<20} {:<16} {}",
            g.id,
            g.name,
            state_text(out, &s)
        ));
        rows.push(status_json(&s, false));
    }
    Ok(rows)
}

fn print_discovery(r: &bonfire_app::discover::ScanReport, out: &Out) {
    use bonfire_app::discover::FoundState;
    let count = |s: FoundState| r.games.iter().filter(|g| g.state == s).count();
    let platforms = if r.platforms.is_empty() {
        out.t("cli.out.no_platforms").to_string()
    } else {
        r.platforms.join(", ")
    };
    out.line(out.tf(
        "cli.out.discovered",
        &[
            ("platforms", &platforms),
            ("found", &r.games.len()),
            ("new", &count(FoundState::New)),
            ("managed", &count(FoundState::Managed)),
            ("ignored", &count(FoundState::Ignored)),
            ("nosave", &r.without_saves.len()),
        ],
    ));
    for g in &r.games {
        let state = match g.state {
            FoundState::New => out.t("cli.found.new"),
            FoundState::Managed => out.t("cli.found.managed"),
            FoundState::Ignored => out.t("cli.found.ignored"),
        };
        let source = if g.rule.is_some() {
            out.t("cli.found.rules")
        } else {
            out.t("cli.found.guessed")
        };
        out.line(format!(
            "  [{state}] {:<20} {}  {source}  {}",
            g.id,
            g.name,
            out.tf(
                "cli.out.files_total",
                &[("files", &g.files), ("bytes", &human_bytes(g.bytes))]
            )
        ));
    }
    if count(FoundState::New) > 0 {
        out.line(out.t("cli.out.include_hint"));
    }
    if r.rules.as_ref().is_some_and(|l| l.path.is_none()) {
        out.line(out.t("cli.out.no_rules"));
    }
}

#[allow(clippy::too_many_arguments)]
fn info(
    app: &mut App,
    key: &str,
    refresh: bool,
    set: Vec<String>,
    unlock: Vec<String>,
    search: bool,
    pick: Option<String>,
    out: &Out,
) -> Result<()> {
    let g = app.find_game(key)?;
    if search {
        let c = app.search_metadata(&g);
        out.emit(serde_json::to_value(&c).unwrap());
        if c.is_empty() {
            out.line(out.t("cli.out.no_candidates"));
        }
        for x in &c {
            let year = x.year.map(|y| format!(" ({y})")).unwrap_or_default();
            out.line(format!("{}:{}  {}{year}", x.source, x.id, x.name));
        }
        return Ok(());
    }
    for s in &set {
        let (field, value) = s
            .split_once('=')
            .ok_or_else(|| AppError::Invalid(format!("--set 的格式为 字段=值：{s}")))?;
        app.set_metadata_field(&g, field.trim(), value)?;
    }
    for f in &unlock {
        app.unlock_field(&g, f.trim())?;
    }
    if let Some(p) = &pick {
        let (source, id) = p
            .split_once(':')
            .ok_or_else(|| AppError::Invalid(format!("--pick 的格式为 来源:ID：{p}")))?;
        app.pick_metadata(&g, source, id)?;
    } else if refresh {
        app.refresh_metadata(&g, true)?;
    }
    let view = app.metadata(&g.id)?;
    out.emit(json!({ "game": g, "metadata": view }));
    let Some(v) = view else {
        out.line(format!("{} ({})", g.name, g.id));
        out.line(out.t("cli.out.no_metadata"));
        return Ok(());
    };
    let m = &v.data;
    let title = match (&m.name_zh, &m.name) {
        (Some(zh), Some(n)) if zh != n => format!("{zh} / {n}"),
        (Some(x), _) | (None, Some(x)) => x.clone(),
        _ => g.name.clone(),
    };
    out.line(format!("{title} ({})", g.id));
    let row = |k: &str, v: Option<String>| {
        if let Some(v) = v.filter(|s| !s.is_empty()) {
            out.line(format!("  {:<8} {v}", out.t(k)));
        }
    };
    row("cli.meta.developer", m.developer.clone());
    row("cli.meta.publisher", m.publisher.clone());
    row("cli.meta.release", m.release_date.clone());
    row("cli.meta.genres", Some(m.genres.join(", ")));
    row(
        "cli.meta.playtime",
        m.playtime_min.map(|p| {
            out.tf(
                "cli.meta.hours",
                &[("h", &format!("{:.1}", p as f64 / 60.0))],
            )
        }),
    );
    row(
        "cli.meta.cloud",
        m.cloud_save.map(|c| {
            out.t(if c { "cli.meta.yes" } else { "cli.meta.no" })
                .to_string()
        }),
    );
    row("cli.meta.cover", m.cover_path.clone());
    if !m.locked_fields.is_empty() {
        row("cli.meta.locked", Some(m.locked_fields.join(", ")));
    }
    Ok(())
}

fn rules(app: &App, cmd: RulesCmd, out: &Out) -> Result<()> {
    match cmd {
        RulesCmd::Update { bundle, url } => {
            let url = url
                .as_deref()
                .unwrap_or(bonfire_app::discovery::manifest::MANIFEST_URL);
            let r = app.update_rules(url, bundle.as_deref())?;
            out.emit(serde_json::to_value(&r).unwrap());
            match r {
                bonfire_app::discovery::manifest::UpdateOutcome::Updated { games, bytes } => out
                    .line(out.tf(
                        "cli.out.rules_updated",
                        &[("games", &games), ("bytes", &human_bytes(bytes))],
                    )),
                bonfire_app::discovery::manifest::UpdateOutcome::NotModified => {
                    out.line(out.t("cli.out.rules_not_modified"))
                }
            }
        }
        RulesCmd::Info => {
            let (_, loaded) = app.load_rules()?;
            out.emit(serde_json::to_value(&loaded).unwrap());
            let src = loaded
                .path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| out.t("cli.out.no_rules").to_string());
            out.line(out.tf(
                "cli.out.rules_info",
                &[
                    ("games", &loaded.games),
                    ("user", &loaded.user_games),
                    ("source", &src),
                ],
            ));
        }
    }
    Ok(())
}

fn status_one(app: &App, key: &str, force_verify: bool, out: &Out) -> Result<()> {
    let g = app.find_game(key)?;
    let s = game_status(app, &g, force_verify)?;
    out.emit(status_json(&s, true));
    out.line(format!("{} ({})  {}", g.name, g.id, state_text(out, &s)));
    out.line(format!(
        "  {}",
        out.tf(
            "cli.out.files_total",
            &[("files", &s.files), ("bytes", &human_bytes(s.total_bytes))]
        )
    ));
    if s.detached
        && let Some(h) = s.head
    {
        out.line(format!(
            "  {}",
            out.tf(
                "cli.out.detached",
                &[
                    ("id", &h.short(12)),
                    ("action", &out.t("cli.out.detached_action"))
                ]
            )
        ));
    }
    for c in s
        .changes
        .iter()
        .filter(|c| c.status != FileStatus::Unchanged)
    {
        let key = match c.status {
            FileStatus::Added => "cli.file.added",
            FileStatus::Modified => "cli.file.modified",
            _ => "cli.file.deleted",
        };
        out.line(format!("  {:<6} {}", out.t(key), c.path));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn backup(
    app: &mut App,
    games: Vec<String>,
    message: Option<String>,
    profile: Option<String>,
    tag: Option<String>,
    force: bool,
    all: bool,
    on_detached: DetachedChoice,
    out: &Out,
) -> Result<()> {
    let mut comment = message;
    let mut keys = games;
    if let Some(name) = &profile {
        let p = app
            .index
            .profile(name)?
            .ok_or_else(|| AppError::ProfileNotFound(name.clone()))?;
        if comment.is_none() && !p.default_comment.is_empty() {
            comment = Some(p.default_comment.clone());
        }
        keys.extend(p.game_ids);
    }
    let targets: Vec<GameRecord> = if all {
        let mut v = Vec::new();
        for g in app.games()?.into_iter().filter(|g| g.enabled) {
            if !matches!(
                app.status(&g)?.state,
                GameState::UpToDate | GameState::ShotsOnly
            ) {
                v.push(g);
            }
        }
        v
    } else {
        if keys.is_empty() {
            return Err(AppError::Invalid("请指定游戏、--profile 或 --all".into()));
        }
        keys.iter()
            .map(|k| app.find_game(k))
            .collect::<Result<Vec<_>>>()?
    };
    if targets.is_empty() {
        out.emit(json!({ "results": [] }));
        out.line(out.t("cli.out.nothing_to_backup"));
        return Ok(());
    }

    // 游戏正在运行时只提醒，不阻止备份（U-12）。
    let ids: Vec<String> = targets.iter().map(|g| g.id.clone()).collect();
    for (id, procs) in app.running_games(&ids).unwrap_or_default() {
        if let Some(g) = targets.iter().find(|g| g.id == id) {
            out.line(out.tf(
                "cli.out.backup_game_running",
                &[("game", &g.name), ("procs", &procs.join("、"))],
            ));
        }
    }
    let lock = app.repo.lock()?;
    let opts = BackupOptions {
        comment,
        tag,
        force,
        on_detached,
        ..Default::default()
    };
    let mut results = Vec::new();
    let mut first_error = None;
    for g in &targets {
        // 只管理截图的游戏不备份存档，只收集截图。
        if app.is_shots_only(&g.id)? {
            out.line(out.tf("cli.out.backup_shots_only", &[("game", &g.name)]));
            let added = match app.sync_shots(&lock, g) {
                Ok(s) => s.added,
                Err(e) => {
                    out.line(out.tf("cli.out.warning", &[("msg", &e)]));
                    0
                }
            };
            if added > 0 {
                out.line(out.tf("cli.out.shots_collected_n", &[("n", &added)]));
            }
            results.push(
                json!({ "game": g.id, "name": g.name, "result": "shots-only", "shots": added }),
            );
            continue;
        }
        // 单个游戏失败不影响其他游戏（文档 8.1）。
        match app.backup(&lock, g, &opts) {
            Ok(r) => {
                match &r {
                    BackupResult::Created {
                        snapshot,
                        branch,
                        new_branch,
                        comment,
                        warnings,
                        ..
                    } => {
                        out.line(out.tf(
                            "cli.out.backup_created",
                            &[
                                ("game", &g.name),
                                ("id", &snapshot.short(12)),
                                ("branch", branch),
                                ("comment", comment),
                            ],
                        ));
                        if let Some(b) = new_branch {
                            out.line(out.tf("cli.out.new_branch", &[("branch", b)]));
                        }
                        for w in warnings {
                            out.line(out.tf("cli.out.warning", &[("msg", w)]));
                        }
                    }
                    BackupResult::NoChanges => {
                        out.line(out.tf("cli.out.backup_nochange", &[("game", &g.name)]))
                    }
                }
                let mut v = serde_json::to_value(&r).unwrap();
                v["game"] = json!(g.id);
                v["name"] = json!(g.name);
                // 备份时顺带收集新截图（失败只提示，不影响备份结果）。
                match app.sync_shots(&lock, g) {
                    Ok(s) => {
                        if s.added > 0 {
                            out.line(out.tf(
                                "cli.out.shots_collected",
                                &[("n", &s.added), ("size", &human_bytes(s.bytes))],
                            ));
                        }
                        v["shots"] = json!(s.added);
                    }
                    Err(e) => out.line(out.tf("cli.out.warning", &[("msg", &e)])),
                }
                results.push(v);
            }
            Err(e) => {
                if !out.json() {
                    eprintln!(
                        "{}",
                        out.tf("cli.out.failed", &[("game", &g.name), ("msg", &e)])
                    );
                }
                results.push(json!({ "game": g.id, "name": g.name, "result": "error", "error": e.to_string() }));
                first_error.get_or_insert(e);
            }
        }
    }
    out.emit(json!({ "results": results }));
    first_error.map_or(Ok(()), Err)
}

/// `backup --auto`：与桌面程序的定时备份相同，供系统计划任务调用。
fn auto_backup(app: &mut App, out: &Out) -> Result<()> {
    let lock = app.repo.lock()?;
    let r = app.run_auto_backup(&lock, bonfire_app::auto::AutoKind::Scheduled)?;
    out.emit(serde_json::to_value(&r).unwrap());
    for i in &r.items {
        if let Some(id) = &i.snapshot {
            out.line(out.tf(
                "cli.out.auto_created",
                &[("game", &i.name), ("id", &id.short(12))],
            ));
        }
        if let Some(e) = &i.error {
            out.line(out.tf("cli.out.failed", &[("game", &i.name), ("msg", e)]));
        }
        if i.shots > 0 {
            out.line(out.tf("cli.out.shots_collected_n", &[("n", &i.shots)]));
        }
    }
    out.line(out.tf("cli.out.auto_done", &[("n", &r.created())]));
    Ok(())
}

fn log(
    app: &App,
    key: &str,
    branch: Option<&str>,
    grep: Option<&str>,
    limit: usize,
    out: &Out,
) -> Result<()> {
    let g = app.find_game(key)?;
    let entries = app.log(&g, branch, grep, limit)?;
    out.emit(serde_json::to_value(&entries).unwrap());
    if entries.is_empty() {
        out.line(out.t("cli.out.no_snapshots"));
    }
    for e in &entries {
        let marker = if e.head { "*" } else { " " };
        let refs = if e.refs.is_empty() {
            String::new()
        } else {
            format!(" ({})", e.refs.join(", "))
        };
        let trigger = out.t(&format!("cli.trigger.{}", e.trigger));
        let first_line = e.comment.lines().next().unwrap_or("");
        out.line(format!(
            "{marker} {}  {}  [{}]{refs}  {first_line}",
            e.id.short(12),
            e.created_at[..16.min(e.created_at.len())].replace('T', " "),
            trigger
        ));
    }
    Ok(())
}

fn diff(app: &App, key: &str, a: &str, b: Option<&str>, out: &Out) -> Result<()> {
    let g = app.find_game(key)?;
    let d = app.diff(&g, a, b)?;
    out.emit(serde_json::to_value(&d).unwrap());
    if d.is_empty() {
        out.line(out.t("cli.out.no_diff"));
    }
    for e in &d {
        let (sign, k) = match e.status {
            DiffStatus::Added => ("+", "cli.file.added"),
            DiffStatus::Removed => ("-", "cli.file.removed"),
            DiffStatus::Modified => ("~", "cli.file.modified"),
        };
        let size = |s: Option<u64>| s.map(human_bytes).unwrap_or_else(|| "—".into());
        out.line(format!(
            "{sign} {:<6} {}  ({} → {})",
            out.t(k),
            e.path,
            size(e.size_before),
            size(e.size_after)
        ));
    }
    Ok(())
}

fn print_restore(report: &RestoreReport, out: &Out) {
    let p = &report.plan;
    out.emit(serde_json::to_value(report).unwrap());
    if !p.running.is_empty() {
        out.line(out.tf("cli.out.running", &[("p", &p.running.join("、"))]));
    }
    out.line(out.tf(
        "cli.out.restore_plan",
        &[
            ("overwrite", &p.count(Action::Overwrite)),
            ("create", &p.count(Action::Create)),
            ("delete", &p.count(Action::Delete)),
            ("unchanged", &p.unchanged),
        ],
    ));
    for f in &p.files {
        let k = match f.action {
            Action::Overwrite => "cli.file.modified",
            Action::Create => "cli.file.added",
            Action::Delete => "cli.file.deleted",
        };
        out.line(format!("  {:<6} {}", out.t(k), f.path));
    }
}

fn restore(app: &mut App, key: &str, target: &str, opts: RestoreOptions, out: &Out) -> Result<()> {
    let g = app.find_game(key)?;
    if opts.dry_run {
        let lock = app.repo.lock()?;
        let r = app.restore(&lock, &g, target, &opts)?;
        print_restore(&r, out);
        out.line(out.t("cli.out.dry_run"));
        return Ok(());
    }
    let id = app.repo.resolve(&g.id, target)?;
    if !out.confirm(out.tf(
        "cli.confirm.restore",
        &[("game", &g.name), ("target", &id.short(12))],
    ))? {
        out.line(out.t("cli.out.cancelled"));
        return Ok(());
    }
    let lock = app.repo.lock()?;
    let r = app.restore(&lock, &g, &id.to_string(), &opts)?;
    print_restore(&r, out);
    if let Some(p) = r.protection {
        out.line(out.tf("cli.out.protection", &[("id", &p.short(12))]));
    }
    out.line(out.tf("cli.out.restore_done", &[("id", &id.short(12))]));
    Ok(())
}

fn export(
    app: &mut App,
    key: &str,
    snapshot: Option<&str>,
    zip: Option<PathBuf>,
    folder: Option<PathBuf>,
    out: &Out,
) -> Result<()> {
    let g = app.find_game(key)?;
    let (dest, format) = match (zip, folder) {
        (Some(z), _) => (z, ExportFormat::Zip),
        (None, Some(d)) => (d, ExportFormat::Folder),
        (None, None) => {
            return Err(AppError::Invalid(
                out.t("cli.error.export_where").to_string(),
            ));
        }
    };
    let r = app.export_files(&g, snapshot, &dest, format)?;
    out.emit(serde_json::to_value(&r).unwrap());
    let what = match r.snapshot {
        Some(id) => out.tf("cli.out.export_snapshot", &[("id", &id.short(12))]),
        None => out.t("cli.out.export_current").to_string(),
    };
    out.line(out.tf(
        "cli.out.exported",
        &[
            ("what", &what),
            ("files", &r.files),
            ("bytes", &human_bytes(r.bytes)),
            ("dest", &r.dest.display()),
        ],
    ));
    Ok(())
}

fn import(app: &mut App, key: &str, src: &Path, opts: ImportFilesOptions, out: &Out) -> Result<()> {
    let g = app.find_game(key)?;
    let lock = app.repo.lock()?;
    let r = app.import_files(&lock, &g, src, &opts)?;
    out.emit(serde_json::to_value(&r).unwrap());
    let p = &r.preview;
    if let Some(m) = &p.manifest {
        out.line(out.tf(
            "cli.out.import_from",
            &[
                ("game", &m.game_name),
                ("host", &m.host),
                ("time", &m.exported_at),
            ],
        ));
    }
    for f in &p.folders {
        let to = match f.ord {
            Some(o) => out.tf("cli.out.import_location", &[("n", &(o + 1))]),
            None => out.t("cli.out.import_unmapped").to_string(),
        };
        let folder = if f.folder.is_empty() {
            "."
        } else {
            f.folder.as_str()
        };
        out.line(format!(
            "  {folder:<24} -> {to}  ({})",
            out.tf("cli.out.files_n", &[("n", &f.files)])
        ));
    }
    if let Some(id) = p.identical {
        out.line(out.tf("cli.out.import_identical", &[("id", &id.short(12))]));
    } else if r.dry_run {
        out.line(out.t("cli.out.dry_run"));
    } else if let (Some(id), Some(b)) = (r.snapshot, &r.branch) {
        out.line(out.tf("cli.out.imported", &[("id", &id.short(12)), ("branch", b)]));
        out.line(out.t("cli.out.import_hint"));
    }
    Ok(())
}

fn bundle(app: &mut App, cmd: BundleCmd, out: &Out) -> Result<()> {
    match cmd {
        BundleCmd::Export {
            games,
            all,
            out_file,
            branches,
            from,
            to,
            shots,
            no_saves,
        } => {
            // 多个游戏（或 --all）、或只导出截图时，导出为篝火包（合集包格式），每个游戏全部分支。
            if all || games.len() > 1 || no_saves {
                if !branches.is_empty() || to.is_some() {
                    return Err(AppError::Invalid(
                        out.t("cli.error.collection_scope").into(),
                    ));
                }
                let list = if all {
                    app.games()?
                } else {
                    games
                        .iter()
                        .map(|k| app.find_game(k))
                        .collect::<Result<Vec<_>>>()?
                };
                let r = app.export_package(
                    &list,
                    &out_file,
                    &PackageOptions {
                        saves: !no_saves,
                        shots,
                        shot_ids: Default::default(),
                    },
                )?;
                out.emit(serde_json::to_value(&r).unwrap());
                out.line(out.tf(
                    "cli.out.collection_exported",
                    &[
                        ("games", &r.games.len()),
                        ("bytes", &human_bytes(r.bytes)),
                        ("dest", &r.dest.display()),
                    ],
                ));
                for e in &r.games {
                    let mut line = out.tf("cli.out.snapshots_n", &[("n", &e.snapshots)]);
                    if e.shots > 0 {
                        line.push_str(&out.tf("cli.out.package_shots_n", &[("n", &e.shots)]));
                    }
                    out.line(format!("  {:<28} {}", e.game_name, line));
                }
                if !r.skipped.is_empty() {
                    out.line(out.tf(
                        "cli.out.collection_skipped",
                        &[("games", &r.skipped.join("、"))],
                    ));
                }
                return Ok(());
            }
            let g = app.find_game(&games[0])?;
            let scope = match (to, branches.is_empty()) {
                (Some(to), _) => BundleScope::Range { from, to },
                (None, false) => BundleScope::Branches { names: branches },
                (None, true) => BundleScope::All,
            };
            let r = app.export_bundle(&g, &scope, &out_file, shots)?;
            out.emit(serde_json::to_value(&r).unwrap());
            let refs: Vec<String> = r
                .refs
                .iter()
                .filter(|x| x.kind == RefKind::Branch)
                .map(|x| x.name.clone())
                .collect();
            out.line(out.tf(
                "cli.out.bundle_exported",
                &[
                    ("snapshots", &r.snapshots),
                    ("branches", &refs.join(", ")),
                    ("bytes", &human_bytes(r.bytes)),
                    ("dest", &r.dest.display()),
                ],
            ));
            if r.prerequisites > 0 {
                out.line(out.tf("cli.out.bundle_partial", &[("n", &r.prerequisites)]));
            }
            Ok(())
        }
        BundleCmd::Import {
            file,
            game,
            only,
            dry_run,
            no_saves,
            no_shots,
            shots_to_disk,
            adopt_main,
        } => {
            let lock = app.repo.lock()?;
            let preview = app.inspect_archive(&file)?;
            let partial = no_saves || no_shots || shots_to_disk || adopt_main;
            if !preview.collection && only.is_empty() && !partial {
                let r = app.import_bundle(&lock, &file, game.as_deref(), dry_run)?;
                out.emit(serde_json::to_value(&r).unwrap());
                print_bundle_import(&r, dry_run, out);
                return Ok(());
            }
            // 合集包：导入全部或 --only 指定的游戏（按游戏 ID 或名称）。
            let mut picks = Vec::new();
            for key in &only {
                let lower = key.to_lowercase();
                let hit = preview
                    .games
                    .iter()
                    .find(|g| {
                        g.preview.game_id == *key || g.preview.game_name.to_lowercase() == lower
                    })
                    .ok_or_else(|| {
                        AppError::Invalid(out.tf("cli.error.not_in_collection", &[("game", key)]))
                    })?;
                picks.push(ArchivePick::all(hit.key.clone()));
            }
            // 只导入存档或截图（U-16）。
            if partial {
                if picks.is_empty() {
                    picks = preview
                        .games
                        .iter()
                        .map(|g| ArchivePick {
                            target: game.clone(),
                            ..ArchivePick::all(g.key.clone())
                        })
                        .collect();
                }
                for p in &mut picks {
                    p.saves = !no_saves;
                    p.shots = !no_shots;
                    p.adopt_main = adopt_main;
                }
            }
            let items = app.import_archive(&lock, &file, &picks, dry_run)?;
            out.emit(serde_json::to_value(&items).unwrap());
            out.line(out.tf(
                "cli.out.collection_info",
                &[
                    ("host", &preview.host),
                    ("time", &preview.created_at),
                    ("games", &preview.games.len()),
                ],
            ));
            let mut failed = 0;
            for item in &items {
                out.line(String::new());
                match (&item.report, &item.error) {
                    (Some(r), _) => {
                        print_bundle_import(r, dry_run, out);
                        if let Some(b) = &item.adopted_main {
                            out.line(out.tf(
                                "cli.out.adopted_main",
                                &[("branch", b), ("game", &item.game_name)],
                            ));
                        }
                        // 把新导入的截图写到本机这个游戏的截图文件夹。
                        if shots_to_disk && !dry_run && !r.new_shot_ids.is_empty() {
                            let g = app.find_game(&r.preview.game_id)?;
                            let w = app.sync_shots_to_disk(&g, &r.new_shot_ids)?;
                            out.line(out.tf(
                                "cli.out.shots_written",
                                &[
                                    ("n", &w.written),
                                    ("size", &human_bytes(w.bytes)),
                                    ("exists", &w.exists),
                                    ("nodir", &w.no_dir),
                                ],
                            ));
                        }
                    }
                    (None, Some(e)) => {
                        failed += 1;
                        out.line(out.tf(
                            "cli.out.collection_failed",
                            &[("game", &item.game_name), ("error", e)],
                        ));
                    }
                    _ => {}
                }
            }
            if failed > 0 {
                return Err(AppError::Invalid(
                    out.tf("cli.error.collection_failed", &[("n", &failed)]),
                ));
            }
            Ok(())
        }
    }
}

fn print_bundle_import(r: &BundleImportReport, dry_run: bool, out: &Out) {
    let p = &r.preview;
    out.line(out.tf(
        "cli.out.bundle_info",
        &[
            ("game", &p.game_name),
            ("host", &p.host),
            ("time", &p.created_at),
            ("total", &p.nodes.len()),
            ("new", &p.new_snapshots),
        ],
    ));
    for x in &p.refs {
        let kind = if x.kind == RefKind::Tag {
            out.t("cli.ref.tag")
        } else {
            out.t("cli.ref.branch")
        };
        let action = match x.action {
            RefAction::Create => out.t("cli.bundle.create").to_string(),
            RefAction::Same => out.t("cli.bundle.same").to_string(),
            RefAction::FastForward => out.t("cli.bundle.fast_forward").to_string(),
            RefAction::Contained => out.t("cli.bundle.contained").to_string(),
            RefAction::Diverged => out.tf(
                "cli.bundle.diverged",
                &[
                    ("name", &x.new_name.clone().unwrap_or_default()),
                    (
                        "fork",
                        &x.fork_point
                            .map(|f| f.short(12))
                            .unwrap_or_else(|| "-".into()),
                    ),
                ],
            ),
            RefAction::TagConflict => out.t("cli.bundle.tag_conflict").to_string(),
        };
        out.line(format!("  {kind} {:<20} {action}", x.name));
    }
    if p.location_mismatch {
        out.line(out.t("cli.out.bundle_locations"));
    }
    if r.created_game {
        out.line(out.tf("cli.out.bundle_new_game", &[("game", &p.game_name)]));
    }
    if dry_run {
        out.line(out.t("cli.out.dry_run"));
    } else {
        out.line(out.tf("cli.out.bundle_imported", &[("n", &r.imported_snapshots)]));
        if r.imported_shots > 0 {
            out.line(out.tf("cli.out.bundle_shots", &[("n", &r.imported_shots)]));
        }
    }
}

fn delete_branch(app: &mut App, key: &str, name: &str, dry_run: bool, out: &Out) -> Result<()> {
    let g = app.find_game(key)?;
    let lock = app.repo.lock()?;
    let plan = app.delete_branch(&lock, &g, name, true)?;
    if !dry_run
        && !out.confirm(out.tf(
            "cli.confirm.delete_branch",
            &[("name", &name), ("n", &plan.removed.len())],
        ))?
    {
        out.line(out.t("cli.out.cancelled"));
        return Ok(());
    }
    let r = if dry_run {
        plan
    } else {
        app.delete_branch(&lock, &g, name, false)?
    };
    out.emit(serde_json::to_value(&r).unwrap());
    out.line(out.tf(
        "cli.out.branch_deleted",
        &[("name", &name), ("n", &r.removed.len())],
    ));
    if let Some(b) = &r.head_branch {
        out.line(out.tf("cli.out.head_switched", &[("name", b)]));
    }
    if dry_run {
        out.line(out.t("cli.out.dry_run"));
    }
    Ok(())
}

fn delete_snapshot(app: &mut App, key: &str, spec: &str, dry_run: bool, out: &Out) -> Result<()> {
    let g = app.find_game(key)?;
    let lock = app.repo.lock()?;
    let plan = app.delete_snapshot(&lock, &g, spec, true)?;
    let id = plan.removed[0].short(12);
    if !dry_run
        && !out.confirm(out.tf(
            "cli.confirm.delete_snapshot",
            &[("id", &id), ("n", &plan.rewritten)],
        ))?
    {
        out.line(out.t("cli.out.cancelled"));
        return Ok(());
    }
    let r = if dry_run {
        plan
    } else {
        app.delete_snapshot(&lock, &g, spec, false)?
    };
    out.emit(serde_json::to_value(&r).unwrap());
    out.line(out.tf(
        "cli.out.snapshot_deleted",
        &[("id", &id), ("n", &r.rewritten)],
    ));
    for b in &r.branches_deleted {
        out.line(out.tf("cli.out.branch_emptied", &[("name", b)]));
    }
    if let Some(b) = &r.head_branch {
        out.line(out.tf("cli.out.head_switched", &[("name", b)]));
    }
    if dry_run {
        out.line(out.t("cli.out.dry_run"));
    }
    Ok(())
}

fn undo(app: &mut App, key: &str, out: &Out) -> Result<()> {
    let g = app.find_game(key)?;
    if !out.confirm(out.tf("cli.confirm.undo", &[("game", &g.name)]))? {
        out.line(out.t("cli.out.cancelled"));
        return Ok(());
    }
    let lock = app.repo.lock()?;
    let r = app.undo_restore(&lock, &g)?;
    print_restore(&r, out);
    out.line(out.tf("cli.out.undo_done", &[("id", &r.plan.target.short(12))]));
    Ok(())
}

fn refs(
    app: &mut App,
    key: &str,
    kind: RefKind,
    name: Option<String>,
    snapshot: Option<String>,
    out: &Out,
) -> Result<()> {
    let g = app.find_game(key)?;
    match name {
        Some(n) => {
            let lock = app.repo.lock()?;
            let id = app.create_ref(&lock, &g, kind, &n, snapshot.as_deref())?;
            let full = match kind {
                RefKind::Branch => format!("heads/{n}"),
                RefKind::Tag => format!("tags/{n}"),
            };
            out.emit(json!({ "ref": full, "snapshot": id }));
            out.line(out.tf(
                "cli.out.ref_created",
                &[("name", &full), ("id", &id.short(12))],
            ));
        }
        None => {
            let head = app.repo.head(&g.id)?;
            let list = app.refs(&g)?;
            out.emit(Value::Array(
                list.iter()
                    .map(|(r, id)| json!({ "ref": r.to_string(), "snapshot": id }))
                    .collect(),
            ));
            for (r, id) in &list {
                let current =
                    r.kind == RefKind::Branch && r.name == head.branch && head.detached.is_none();
                out.line(format!(
                    "{} {:<24} {}",
                    if current { "*" } else { " " },
                    r.to_string(),
                    id.short(12)
                ));
            }
        }
    }
    Ok(())
}

fn profile(app: &mut App, cmd: ProfileCmd, out: &Out) -> Result<()> {
    match cmd {
        ProfileCmd::List => {
            let list = app.index.profiles()?;
            out.emit(serde_json::to_value(&list).unwrap());
            if list.is_empty() {
                out.line(out.t("cli.out.no_profiles"));
            }
            for p in &list {
                out.line(format!("{:<16} {}", p.name, p.game_ids.join(", ")));
            }
        }
        ProfileCmd::Save {
            name,
            games,
            message,
        } => {
            let ids = games
                .iter()
                .map(|k| app.find_game(k).map(|g| g.id))
                .collect::<Result<Vec<_>>>()?;
            let p = Profile {
                name: name.trim().to_string(),
                game_ids: ids,
                default_comment: message.unwrap_or_default(),
            };
            if p.name.is_empty() {
                return Err(AppError::Invalid("方案名称不能为空".into()));
            }
            app.index.save_profile(&p)?;
            out.emit(serde_json::to_value(&p).unwrap());
            out.line(out.tf(
                "cli.out.profile_saved",
                &[("name", &p.name), ("n", &p.game_ids.len())],
            ));
        }
        ProfileCmd::Rm { name } => {
            if !app.index.delete_profile(&name)? {
                return Err(AppError::ProfileNotFound(name));
            }
            out.emit(json!({ "removed": name }));
            out.line(out.tf("cli.out.profile_removed", &[("name", &name)]));
        }
    }
    Ok(())
}

fn fsck(app: &App, deep: bool, out: &Out) -> Result<()> {
    let r = app.fsck(deep)?;
    out.emit(json!({ "ok": r.ok(), "report": r }));
    if r.ok() {
        out.line(out.tf(
            "cli.out.fsck_ok",
            &[
                ("chunks", &r.chunks),
                ("blobs", &r.blobs),
                ("trees", &r.trees),
                ("snapshots", &r.snapshots),
                ("refs", &r.refs),
            ],
        ));
    } else {
        // 问题清单在任何模式下都写到 stderr。
        eprintln!(
            "{}",
            out.tf("cli.out.fsck_errors", &[("n", &r.errors.len())])
        );
        for e in &r.errors {
            eprintln!("  {e}");
        }
    }
    if r.unreachable_snapshots > 0 {
        out.line(out.tf(
            "cli.out.fsck_unreachable",
            &[("n", &r.unreachable_snapshots)],
        ));
    }
    if r.tmp_files > 0 {
        out.line(out.tf("cli.out.fsck_tmp", &[("n", &r.tmp_files)]));
    }
    if r.ok() {
        Ok(())
    } else {
        Err(AppError::RepoDamaged(r.errors.len()))
    }
}

fn shots(app: &mut App, cmd: ShotsCmd, out: &Out) -> Result<()> {
    match cmd {
        ShotsCmd::List { game } => {
            let g = app.find_game(&game)?;
            let album = app.album(&g)?;
            out.emit(serde_json::to_value(&album).unwrap());
            if album.shots_only {
                out.line(out.tf("cli.out.shots_only_game", &[("game", &g.name)]));
            }
            out.line(out.tf(
                "cli.out.shots_summary",
                &[
                    ("game", &g.name),
                    ("n", &album.shots.len()),
                    ("size", &human_bytes(album.bytes)),
                    ("pending", &album.pending),
                ],
            ));
            if !album.enabled {
                out.line(out.t("cli.out.shots_disabled"));
            }
            out.line(out.tf(
                "cli.out.shots_where",
                &[("dir", &album.dir.display()), ("trash", &album.trash.len())],
            ));
            for s in &album.shots {
                let name = Path::new(&s.original_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                out.line(format!(
                    "{}  {}  {:<8} {:>9}  {name}",
                    s.blob.short(12),
                    s.taken_at
                        .get(..16)
                        .unwrap_or(&s.taken_at)
                        .replace('T', " "),
                    out.t(&format!("cli.shot_source.{}", s.source)),
                    human_bytes(s.size)
                ));
            }
            Ok(())
        }
        ShotsCmd::Sync { games } => {
            let lock = app.repo.lock()?;
            let reports = if games.is_empty() {
                app.sync_all_shots(&lock)?
            } else {
                let mut v = Vec::new();
                for k in &games {
                    let g = app.find_game(k)?;
                    v.push(app.sync_shots(&lock, &g)?);
                }
                v
            };
            out.emit(serde_json::to_value(&reports).unwrap());
            let total: usize = reports.iter().map(|r| r.added).sum();
            for r in reports.iter().filter(|r| r.added > 0) {
                let name = app
                    .find_game(&r.game_id)
                    .map(|g| g.name)
                    .unwrap_or_default();
                out.line(out.tf(
                    "cli.out.shots_synced_game",
                    &[
                        ("game", &name),
                        ("n", &r.added),
                        ("size", &human_bytes(r.bytes)),
                    ],
                ));
            }
            out.line(out.tf("cli.out.shots_synced", &[("n", &total)]));
            Ok(())
        }
        ShotsCmd::Export {
            game,
            out_file,
            folder,
            ids,
            since,
            snapshot,
        } => {
            let g = app.find_game(&game)?;
            let mut chosen = app.resolve_shots(&g, &ids)?;
            if since.is_some() || snapshot.is_some() {
                let all = app.album(&g)?.shots;
                let map = app.shot_snapshots(&g)?;
                let snap = match &snapshot {
                    Some(s) => Some(app.repo.resolve(&g.id, s)?),
                    None => None,
                };
                let filtered: Vec<_> = all
                    .iter()
                    .filter(|s| since.as_deref().is_none_or(|d| s.taken_at.as_str() >= d))
                    .filter(|s| {
                        snap.is_none_or(|id| map.get(&s.blob).copied().flatten() == Some(id))
                    })
                    .filter(|s| chosen.is_empty() || chosen.contains(&s.blob))
                    .map(|s| s.blob)
                    .collect();
                if filtered.is_empty() {
                    return Err(AppError::Invalid(out.t("cli.error.no_shots_match").into()));
                }
                chosen = filtered;
            }
            let format = if folder {
                ExportFormat::Folder
            } else {
                ExportFormat::Zip
            };
            let r = app.export_shots(&g, &chosen, &out_file, format)?;
            out.emit(serde_json::to_value(&r).unwrap());
            out.line(out.tf(
                "cli.out.shots_exported",
                &[
                    ("n", &r.files),
                    ("size", &human_bytes(r.bytes)),
                    ("dest", &r.dest.display()),
                ],
            ));
            Ok(())
        }
        ShotsCmd::Rm {
            game,
            ids,
            purge,
            delete_originals,
        } => {
            let g = app.find_game(&game)?;
            let blobs = app.resolve_shots(&g, &ids)?;
            let key = if purge {
                "cli.confirm.shots_purge"
            } else {
                "cli.confirm.shots_rm"
            };
            let mut text = out.tf(key, &[("n", &blobs.len()), ("game", &g.name)]);
            if delete_originals {
                text.push_str(out.t("cli.confirm.shots_delete_originals"));
            }
            if !out.confirm(text)? {
                out.line(out.t("cli.out.cancelled"));
                return Ok(());
            }
            let lock = app.repo.lock()?;
            let (n, report) = if purge {
                let r = app.purge_shots_with(&lock, &g, &blobs, delete_originals)?;
                (r.purged, Some(r))
            } else {
                (app.delete_shots(&lock, &g, &blobs)?, None)
            };
            out.emit(json!({ "deleted": n, "purged": purge, "originals": report }));
            if let Some(r) = report.as_ref().filter(|_| delete_originals) {
                out.line(originals_line(out, r));
            }
            out.line(out.tf(
                if purge {
                    "cli.out.shots_purged"
                } else {
                    "cli.out.shots_deleted"
                },
                &[("n", &n)],
            ));
            Ok(())
        }
        ShotsCmd::Restore { game, ids } => {
            let g = app.find_game(&game)?;
            let blobs = app.resolve_shots(&g, &ids)?;
            let lock = app.repo.lock()?;
            let n = app.restore_shots(&lock, &g, &blobs)?;
            out.emit(json!({ "restored": n }));
            out.line(out.tf("cli.out.shots_restored", &[("n", &n)]));
            Ok(())
        }
        ShotsCmd::Trash {
            game,
            empty,
            delete_originals,
        } => {
            let g = app.find_game(&game)?;
            if empty {
                let n = app.album(&g)?.trash.len();
                let mut text = out.tf(
                    "cli.confirm.shots_empty_trash",
                    &[("n", &n), ("game", &g.name)],
                );
                if delete_originals {
                    text.push_str(out.t("cli.confirm.shots_delete_originals"));
                }
                if n > 0 && !out.confirm(text)? {
                    out.line(out.t("cli.out.cancelled"));
                    return Ok(());
                }
                let lock = app.repo.lock()?;
                let r = app.purge_shots_with(&lock, &g, &[], delete_originals)?;
                out.emit(serde_json::to_value(&r).unwrap());
                out.line(out.tf("cli.out.shots_purged", &[("n", &r.purged)]));
                if delete_originals {
                    out.line(originals_line(out, &r));
                }
                return Ok(());
            }
            let album = app.album(&g)?;
            out.emit(serde_json::to_value(&album.trash).unwrap());
            out.line(out.tf(
                "cli.out.shots_trash",
                &[
                    ("game", &g.name),
                    ("n", &album.trash.len()),
                    ("size", &human_bytes(album.trash_bytes)),
                ],
            ));
            for s in &album.trash {
                out.line(format!(
                    "{}  {}  {}",
                    s.blob.short(12),
                    s.taken_at
                        .get(..16)
                        .unwrap_or(&s.taken_at)
                        .replace('T', " "),
                    Path::new(&s.original_path)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                ));
            }
            Ok(())
        }
        ShotsCmd::Found => {
            let c = app.shot_candidates()?;
            out.emit(serde_json::to_value(&c).unwrap());
            if c.is_empty() {
                out.line(out.t("cli.out.shots_found_none"));
            }
            for x in &c {
                out.line(format!(
                    "{:<20} {:<28} {:>4}  {}",
                    x.key,
                    x.name,
                    x.count,
                    x.dir.display()
                ));
            }
            Ok(())
        }
        ShotsCmd::Diff { game } => {
            let g = app.find_game(&game)?;
            let d = app.shot_diff(&g)?;
            out.emit(serde_json::to_value(&d).unwrap());
            let trash = d.ignored.iter().filter(|i| !i.purged).count();
            out.line(out.tf(
                "cli.out.shots_diff",
                &[
                    ("game", &g.name),
                    ("synced", &d.synced),
                    ("new", &d.new.len()),
                    ("dup", &d.duplicates),
                    ("trash", &trash),
                    ("purged", &(d.ignored.len() - trash)),
                    ("missing", &d.missing.len()),
                    ("restorable", &d.restorable),
                ],
            ));
            for f in &d.new {
                out.line(format!("  + {}", f.path.display()));
            }
            for i in &d.ignored {
                out.line(format!(
                    "  {} {}",
                    if i.purged { "x" } else { "~" },
                    i.path.display()
                ));
            }
            for s in &d.missing {
                out.line(format!("  - {}  {}", s.blob.short(12), s.original_path));
            }
            Ok(())
        }
        ShotsCmd::WriteBack { game, ids, to } => {
            let g = app.find_game(&game)?;
            let blobs = app.resolve_shots(&g, &ids)?;
            let n = if blobs.is_empty() {
                app.shot_diff(&g)?.missing.len()
            } else {
                blobs.len()
            };
            if n == 0 {
                out.emit(json!({ "written": 0 }));
                out.line(out.t("cli.out.shots_nothing_missing"));
                return Ok(());
            }
            let place = match &to {
                Some(d) => d.display().to_string(),
                None => out.t("cli.out.shots_original_place").to_string(),
            };
            if !out.confirm(out.tf(
                "cli.confirm.shots_write_back",
                &[("n", &n), ("game", &g.name), ("place", &place)],
            ))? {
                out.line(out.t("cli.out.cancelled"));
                return Ok(());
            }
            let r = app.write_back_shots(&g, &blobs, to.as_deref())?;
            out.emit(serde_json::to_value(&r).unwrap());
            out.line(out.tf(
                "cli.out.shots_written",
                &[
                    ("n", &r.written),
                    ("size", &human_bytes(r.bytes)),
                    ("exists", &r.exists),
                    ("nodir", &r.no_dir),
                ],
            ));
            Ok(())
        }
        ShotsCmd::Recollect { game } => {
            let g = app.find_game(&game)?;
            let lock = app.repo.lock()?;
            let n = app.recollect_shots(&lock, &g, &[])?;
            out.emit(json!({ "recollected": n }));
            out.line(out.tf("cli.out.shots_recollected", &[("n", &n)]));
            Ok(())
        }
        ShotsCmd::Set { game, value } => {
            let g = app.find_game(&game)?;
            let v = match value {
                ShotSwitch::On => Some(true),
                ShotSwitch::Off => Some(false),
                ShotSwitch::Auto => None,
            };
            app.set_shots_enabled(&g, v)?;
            let on = app.shots_enabled(&g)?;
            out.emit(json!({ "game": g.id, "setting": v, "enabled": on }));
            out.line(out.tf(
                if on {
                    "cli.out.shots_on"
                } else {
                    "cli.out.shots_off"
                },
                &[("game", &g.name)],
            ));
            Ok(())
        }
    }
}

fn mode_of(m: ModeArg) -> ManageMode {
    match m {
        ModeArg::Both => ManageMode::Both,
        ModeArg::Saves => ManageMode::Saves,
        ModeArg::Shots => ManageMode::Shots,
    }
}

fn originals_line(out: &Out, r: &bonfire_app::shots::PurgeReport) -> String {
    out.tf(
        "cli.out.shots_originals",
        &[
            ("n", &r.originals_deleted),
            ("kept", &r.originals_kept),
            ("failed", &r.originals_failed),
        ],
    )
}

pub(crate) fn shots_dir(app: &mut App, cmd: ShotsDirCmd, out: &Out) -> Result<()> {
    match cmd {
        ShotsDirCmd::Add { game, dir } => {
            let g = app.find_game(&game)?;
            let d = app.add_shot_dir(&g, &dir)?;
            out.emit(json!({ "game": g.id, "dir": d }));
            out.line(out.tf("cli.out.shots_dir_added", &[("game", &g.name), ("dir", &d)]));
        }
        ShotsDirCmd::Rm { game, dir } => {
            let g = app.find_game(&game)?;
            if !app.remove_shot_dir(&g, &dir)? {
                return Err(AppError::Invalid(
                    out.tf("cli.error.no_shots_dir", &[("dir", &dir)]),
                ));
            }
            out.emit(json!({ "game": g.id, "removed": dir }));
            out.line(out.tf(
                "cli.out.shots_dir_removed",
                &[("game", &g.name), ("dir", &dir)],
            ));
        }
    }
    Ok(())
}

/// 导入下载的存档（U-42）：自动对应存档位置，叠加（默认）或整体替换，存为新路线。
#[allow(clippy::too_many_arguments)]
fn import_download(
    app: &mut App,
    game: &str,
    source: &Path,
    branch: Option<String>,
    message: Option<String>,
    password: Option<String>,
    replace: bool,
    dry_run: bool,
    out: &Out,
) -> Result<()> {
    use bonfire_app::downloads::{DownloadImport, DownloadKind, DownloadMode};
    let g = app.find_game(game)?;
    let p = app.inspect_download(&g, source, password.as_deref())?;
    match p.kind {
        DownloadKind::Download => {}
        DownloadKind::NeedsPassword => {
            return Err(AppError::Invalid(
                out.t("cli.error.download_password").into(),
            ));
        }
        DownloadKind::NeedsExtract => {
            return Err(AppError::Invalid(
                out.t("cli.error.download_extract").into(),
            ));
        }
        DownloadKind::Bonfire => {
            return Err(AppError::Invalid(
                out.t("cli.error.download_bonfire").into(),
            ));
        }
        DownloadKind::Damaged => {
            return Err(AppError::Invalid(
                out.t("cli.error.download_damaged").into(),
            ));
        }
    }
    if let Some(n) = p.notice {
        out.line(out.t(match n {
            bonfire_app::downloads::DownloadNotice::Modified => "cli.out.download_modified",
            bonfire_app::downloads::DownloadNotice::Unverifiable => "cli.out.download_unverifiable",
        }));
    }
    let mapping: Vec<(String, String)> = p
        .files
        .iter()
        .filter(|f| !f.ignored)
        .filter_map(|f| f.target.clone().map(|t| (f.rel.clone(), t)))
        .collect();
    let unmapped: Vec<&str> = p
        .files
        .iter()
        .filter(|f| !f.ignored && f.target.is_none())
        .map(|f| f.rel.as_str())
        .collect();
    for (rel, t) in &mapping {
        out.line(format!("  {rel} → {t}"));
    }
    for rel in &unmapped {
        out.line(out.tf("cli.out.download_unmapped", &[("file", rel)]));
    }
    if dry_run {
        out.emit(serde_json::to_value(&p).unwrap());
        return Ok(());
    }
    if mapping.is_empty() {
        return Err(AppError::Invalid(
            out.t("cli.error.download_nothing").into(),
        ));
    }
    let lock = app.repo.lock()?;
    let r = app.import_download(
        &lock,
        &g,
        source,
        password.as_deref(),
        &DownloadImport {
            mapping,
            mode: if replace {
                DownloadMode::Replace
            } else {
                DownloadMode::Overlay
            },
            branch,
            comment: message,
        },
    )?;
    out.emit(serde_json::to_value(&r).unwrap());
    match (&r.branch, &r.identical) {
        (Some(b), _) => {
            out.line(out.tf("cli.out.download_done", &[("n", &r.files), ("branch", b)]))
        }
        (None, Some(id)) => {
            out.line(out.tf("cli.out.download_identical", &[("id", &id.short(12))]))
        }
        _ => {}
    }
    Ok(())
}

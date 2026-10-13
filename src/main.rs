mod add;
mod add_api_key;
mod adopt;
mod bootstrap;
mod config;
mod doctor;
mod env;
mod handoff;
mod keystore;
mod launch;
mod lifecycle;
mod list;
mod paid;
mod paths;
mod pick;
mod prefs;
mod refresh;
mod resume;
mod schedule;
mod service;
mod shell_init;
mod status;
mod sync;
mod usage;
mod wizard;

use std::ffi::OsString;
use std::io::IsTerminal;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode};
use std::time::SystemTime;

use clap::{CommandFactory, Parser, Subcommand};

use crate::env::Env;
use crate::launch::{Invocation, Pick};

/// The exit code when ccx cannot start claude.
const EXIT_EXEC_FAILED: u8 = 127;

/// Launch Claude Code on the least-used of several accounts.
///
/// `ccx [@name] [claude args...]` starts claude. ccx passes all arguments
/// after `@name` to claude. `ccx -- args...` passes all arguments.
#[derive(Debug, Parser)]
#[command(name = "ccx", version, about, disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: Own,
}

/// The commands that ccx keeps for itself (the reserved first words).
#[derive(Debug, Subcommand)]
enum Own {
    /// Manage accounts.
    Manage {
        #[command(subcommand)]
        command: Manage,
    },
    /// Show the kind, tier and usage of each account.
    ///
    /// Status uses the usage cache when it is fresh. If not, it runs the
    /// `/usage` fetch. If the fetch fails, it uses `cachedUsageUtilization`
    /// from `.claude.json`. The exit code is 0 when at least one account is
    /// usable, 3 when all accounts are limited and 4 when no account is
    /// logged in.
    Status {
        /// Print stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Fetch usage and write the usage cache.
    Usage {
        #[command(subcommand)]
        command: UsageCommand,
    },
    /// Copy a session to a different account and resume it there.
    ///
    /// With no session id, handoff lists the threads for the current
    /// directory from all accounts. On a terminal, you then select a thread
    /// and a target account. `ccx handoff <id> @name` does the handoff with
    /// no prompts. Handoff copies the files. It never moves or changes the
    /// source files. When the target has a different copy, handoff writes a
    /// fork with a new session id.
    Handoff {
        /// The session id, or a unique part of 8 or more characters at its
        /// start.
        id: Option<String>,
        /// The target account, for example `@angel2`.
        target: Option<String>,
        /// List the threads from all directories.
        #[arg(long)]
        all: bool,
        /// Copy only. Do not start claude. Print the session id in the
        /// target account.
        #[arg(long)]
        no_resume: bool,
    },
}

/// `ccx manage` commands.
#[derive(Debug, Subcommand)]
enum Manage {
    /// Find the accounts on this machine and write them into the config.
    ///
    /// On a terminal with no `-y`, `--rename` or `--skip`, adopt starts a
    /// wizard: for each found account, you keep the name, give a different
    /// name or skip it. Then adopt shows a summary before any change.
    Adopt {
        /// Confirm the changes without a prompt.
        #[arg(short, long)]
        yes: bool,
        /// Adopt the found account OLD with the name NEW, for example
        /// `--rename angel2=work`. You can give it more than one time.
        #[arg(long, value_name = "OLD=NEW", value_parser = adopt::parse_rename)]
        rename: Vec<(String, String)>,
        /// Do not adopt the found account NAME. You can give it more than
        /// one time.
        #[arg(long, value_name = "NAME")]
        skip: Vec<String>,
    },
    /// Make a new account: the dir, the sync links, the login and the
    /// config entry.
    ///
    /// Add shows a plan and asks before it changes files. It makes the dir
    /// at mode 0700, runs sync, then runs `claude auth login` in the new
    /// account. It compares the email of the login with `--email`. It
    /// writes the account to `config.toml` and the email only to
    /// `config.local.toml`. Then it shows the kind, the tier and the weight.
    ///
    /// With no NAME on a terminal, add starts a wizard. It asks for the
    /// name, the dir, the login and the email, one step at a time.
    Add {
        /// The account name. It must match `^[a-z0-9][a-z0-9_-]*$`.
        name: Option<String>,
        /// The account dir. The default is `~/.claude-NAME`.
        #[arg(long)]
        dir: Option<String>,
        /// The email to give to `claude auth login --email`.
        #[arg(long)]
        email: Option<String>,
        /// Stop after sync. Do not log in.
        #[arg(long)]
        no_login: bool,
        /// Do not copy the user MCP servers and the preferences of the
        /// base account after the login.
        #[arg(long)]
        no_prefs: bool,
        /// Confirm the plan without a prompt.
        #[arg(short, long)]
        yes: bool,
        /// Make an API-key account: a paid fallback. Add reads the key
        /// with a hidden prompt, or one line of stdin, and keeps it in the
        /// OS secret store as `ccx-NAME`. There is no login.
        #[arg(long, conflicts_with_all = ["email", "no_login", "no_prefs"])]
        api_key: bool,
        /// With `--api-key`: get the key from the stdout of this shell
        /// command at each paid launch. ccx keeps no copy of the key.
        #[arg(long, value_name = "COMMAND", requires = "api_key")]
        key_command: Option<String>,
        /// With `--api-key`: keep the key in `~/.config/ccx/keys/NAME` at
        /// mode 0600, not in the OS secret store.
        #[arg(long, requires = "api_key", conflicts_with = "key_command")]
        key_file: bool,
    },
    /// Log in to an account with `claude auth login`.
    ///
    /// Login gives the email from `config.local.toml` to `claude auth login
    /// --email`. Then it compares the email of the login with that email
    /// and fails when they are different. With no email in
    /// `config.local.toml`, login shows a warning and does not compare.
    Login {
        /// The account to log in to.
        #[arg(required_unless_present = "all")]
        name: Option<String>,
        /// Log in to each account that `claude auth status` shows as not
        /// logged in. Then show a summary for each account.
        #[arg(long, conflicts_with = "name")]
        all: bool,
    },
    /// Log out of an account with `claude auth logout`.
    Logout {
        /// The account to log out of.
        name: String,
        /// Log out without a prompt.
        #[arg(short, long)]
        yes: bool,
    },
    /// Change the entry of an account and validate the config.
    ///
    /// Edit changes only the given fields and keeps the comments and the
    /// order. `--email` goes only into `config.local.toml`. The other
    /// fields go into `config.toml`. With no NAME on a terminal, edit starts
    /// a wizard that shows the current values.
    Edit {
        /// The account to change.
        name: Option<String>,
        /// The expected email of the login.
        #[arg(long)]
        email: Option<String>,
        /// The weight for automatic selection. It must be more than 0.
        #[arg(long)]
        weight: Option<f64>,
        /// Use the account for automatic selection.
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        /// Do not use the account for automatic selection.
        #[arg(long)]
        disable: bool,
    },
    /// Open `config.toml` in `$EDITOR` and validate it after the editor
    /// exits. When the new text is not valid, the old file stays.
    EditConfig,
    /// Remove an account from the config. Keep its dir.
    ///
    /// Rm shows a plan and asks before it changes files. It prints the
    /// keychain item name of the account, so that you can remove the item
    /// yourself. With `--purge`, rm also runs `claude auth logout`,
    /// deletes the credentials file of the account and moves the dir into
    /// `~/.local/state/ccx/trash/`. Rm never deletes the dir. Rm refuses
    /// the base account. With no NAME on a terminal, rm starts a wizard.
    /// Before a purge, the wizard asks you to type the account name.
    Rm {
        /// The account to remove.
        name: Option<String>,
        /// Also log out, delete the credentials file and move the dir into
        /// the trash.
        #[arg(long)]
        purge: bool,
        /// Confirm the plan without a prompt.
        #[arg(short, long)]
        yes: bool,
    },
    /// Set up ccx on a new machine in one command.
    ///
    /// Bootstrap finds `claude`, clones the base `~/.claude` from
    /// `[base].repo`, runs `ccx manage sync --all` and logs in to each
    /// account that is not logged in, with the email check. Then it offers
    /// the service and shows `ccx status` and the shell line. It is safe to
    /// run again: a step that is done already shows OK.
    Bootstrap {
        /// Install the service without a prompt.
        #[arg(long, conflicts_with = "no_service")]
        service: bool,
        /// Do not install the service.
        #[arg(long)]
        no_service: bool,
    },
    /// Show each configured account.
    List {
        /// Print stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print shell code for the profile: completions and account aliases.
    ///
    /// Add one line to the shell profile, for example
    /// `eval "$(ccx manage shell-init zsh)"`, or for fish
    /// `ccx manage shell-init fish | source`.
    ShellInit {
        /// The shell.
        #[arg(value_enum)]
        shell: shell_init::Shell,
    },
    /// Link the shared setup of the base account into other accounts.
    ///
    /// Sync shows a plan: LINK, OK, RELINK, CONFLICT, SKIP and MKDIR. It
    /// moves a CONFLICT item to `<dir>/.ccx-backup/<timestamp>/` only with
    /// `--force` or after you confirm. It never changes the base account.
    ///
    /// With `--prefs`, sync does not link. It copies the user MCP servers
    /// (through `claude mcp add-json -s user`) and the allowlisted keys of
    /// the base `~/.claude.json` into the account.
    Sync {
        /// The account to sync.
        #[arg(required_unless_present = "all")]
        name: Option<String>,
        /// Sync all accounts other than the base account.
        #[arg(long, conflicts_with = "name")]
        all: bool,
        /// Show the plan and change nothing.
        #[arg(long)]
        dry_run: bool,
        /// Move CONFLICT items to the backup without a prompt.
        #[arg(long)]
        force: bool,
        /// Copy the user MCP servers and the allowlisted preferences of
        /// the base account. Do not link.
        #[arg(long, conflicts_with = "force")]
        prefs: bool,
    },
    /// Check the setup and show the problems.
    ///
    /// Doctor only reports. It never changes a file. Doctor prints one line for each check with OK, WARN or FAIL. The exit
    /// code is 0 when all checks pass, 1 for a WARN and 2 for a FAIL.
    Doctor {
        /// Print the results as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Install, remove or show the background usage refresh.
    ///
    /// The service runs `ccx usage refresh --all --background --scheduled`
    /// every 60 seconds: launchd on macOS, a systemd user timer on Linux.
    /// Each run fetches only the accounts that are due. With no systemd,
    /// `install` prints a crontab line and changes no files.
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
}

/// `ccx manage service` commands.
#[derive(Debug, Subcommand)]
enum ServiceCommand {
    /// Write the job for this system and load it.
    Install,
    /// Stop the job and remove its files.
    Uninstall,
    /// Show if the job is installed and loaded, the last refresh, and the
    /// cache age and the next due time of each account.
    Status,
}

/// `ccx usage` commands.
#[derive(Debug, Subcommand)]
enum UsageCommand {
    /// Run the `/usage` fetch and write `~/.local/state/ccx/usage/<name>.json`.
    Refresh {
        /// The account to refresh.
        #[arg(required_unless_present = "all")]
        name: Option<String>,
        /// Refresh all enabled accounts.
        #[arg(long, conflicts_with = "name")]
        all: bool,
        /// Run as the detached background refresh: no output, no wait for
        /// the lock and the `usage.refresh_min_gap_secs` rule.
        #[arg(long, hide = true, requires = "all")]
        background: bool,
        /// Run as the service job: a background refresh that fetches only
        /// the accounts that are due by the adaptive interval.
        #[arg(long, hide = true, requires = "background")]
        scheduled: bool,
    },
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match run(args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("ccx: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<OsString>) -> config::Result<ExitCode> {
    match launch::parse_args(args.clone())? {
        Invocation::Own => run_own(args),
        Invocation::Launch {
            account,
            args,
            subcommand,
            allow_paid,
        } => run_launch(account.as_deref(), subcommand, args, allow_paid),
    }
}

fn run_own(args: Vec<OsString>) -> config::Result<ExitCode> {
    let cli = Cli::parse_from(std::iter::once(OsString::from("ccx")).chain(args));
    match cli.command {
        Own::Manage { command } => {
            let env = Env::from_process();
            match command {
                Manage::Adopt { yes, rename, skip } => {
                    let choices = adopt::Choices {
                        rename: rename.into_iter().collect(),
                        skip: skip.into_iter().collect(),
                    };
                    if !yes && choices.is_empty() && wizard::available() {
                        wizard::finish(adopt::run_wizard(&env, &mut wizard::Terminal))?;
                    } else {
                        adopt::run(&env, &choices, &mut wizard::Ask::Flags { yes })?;
                    }
                }
                Manage::Add {
                    name,
                    dir,
                    email,
                    no_login,
                    no_prefs,
                    yes,
                    api_key,
                    key_command,
                    key_file,
                } => {
                    let args = add::Args {
                        name: name.clone().unwrap_or_default(),
                        dir,
                        email,
                        no_login,
                        no_prefs,
                        yes,
                        api_key,
                        key_command,
                        key_file,
                    };
                    match name {
                        Some(_) => add::run(&env, &args, &mut wizard::Ask::Flags { yes })?,
                        None if wizard::available() => {
                            wizard::finish(add::run_wizard(&env, &args, &mut wizard::Terminal))?
                        }
                        None => {
                            return Err(wizard::no_terminal(
                                "missing the account name NAME",
                                add::USAGE,
                            ));
                        }
                    }
                }
                Manage::Login { name, all: _ } => {
                    if !lifecycle::login(&env, name.as_deref())? {
                        return Ok(ExitCode::FAILURE);
                    }
                }
                Manage::Logout { name, yes } => lifecycle::logout(&env, &name, yes)?,
                Manage::Edit {
                    name,
                    email,
                    weight,
                    enable,
                    disable,
                } => {
                    let edit = lifecycle::Edit {
                        email,
                        weight,
                        enabled: (enable || disable).then_some(enable),
                    };
                    match name {
                        Some(name) => lifecycle::edit(&env, &name, &edit)?,
                        None if wizard::available() => wizard::finish(lifecycle::edit_wizard(
                            &env,
                            &mut wizard::Terminal,
                            &edit,
                        ))?,
                        None => {
                            return Err(wizard::no_terminal(
                                "missing the account name NAME",
                                lifecycle::EDIT_USAGE,
                            ));
                        }
                    }
                }
                Manage::EditConfig => lifecycle::edit_config(&env)?,
                Manage::Rm { name, purge, yes } => match name {
                    Some(name) => {
                        lifecycle::rm(&env, &name, purge, &mut wizard::Ask::Flags { yes })?
                    }
                    None if wizard::available() => {
                        wizard::finish(lifecycle::rm_wizard(&env, purge, &mut wizard::Terminal))?
                    }
                    None => {
                        return Err(wizard::no_terminal(
                            "missing the account name NAME",
                            lifecycle::RM_USAGE,
                        ));
                    }
                },
                Manage::Bootstrap {
                    service,
                    no_service,
                } => {
                    let service = (service || no_service).then_some(service);
                    if !bootstrap::run(&env, &bootstrap::Args { service })? {
                        return Ok(ExitCode::FAILURE);
                    }
                }
                Manage::List { json } => list::run(&env, json)?,
                Manage::ShellInit { shell } => shell_init::run(&env, shell, Cli::command())?,
                Manage::Sync {
                    name,
                    all: _,
                    dry_run,
                    force,
                    prefs,
                } => {
                    let ok = if prefs {
                        prefs::run(&env, name.as_deref(), dry_run)?
                    } else {
                        sync::run(&env, name.as_deref(), dry_run, force)?
                    };
                    if !ok {
                        return Ok(ExitCode::FAILURE);
                    }
                }
                Manage::Doctor { json } => return Ok(ExitCode::from(doctor::run(&env, json)?)),
                Manage::Service { command } => match command {
                    ServiceCommand::Install => service::install(&env)?,
                    ServiceCommand::Uninstall => service::uninstall(&env)?,
                    ServiceCommand::Status => {
                        return Ok(ExitCode::from(service::status(&env)?));
                    }
                },
            }
            Ok(ExitCode::SUCCESS)
        }
        Own::Status { json } => Ok(ExitCode::from(status::run(&Env::from_process(), json)?)),
        Own::Usage {
            command:
                UsageCommand::Refresh {
                    name,
                    all: _,
                    background,
                    scheduled,
                },
        } => {
            let env = Env::from_process();
            let ok = status::refresh(&env, name.as_deref(), background, scheduled)?;
            Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Own::Handoff {
            id,
            target,
            all,
            no_resume,
        } => handoff::run(
            &Env::from_process(),
            handoff::Args {
                id,
                target,
                all,
                no_resume,
            },
        ),
    }
}

fn run_launch(
    name: Option<&str>,
    subcommand: Option<&str>,
    mut args: Vec<OsString>,
    allow_paid: bool,
) -> config::Result<ExitCode> {
    let parent = Env::from_process();
    let depth = launch::check_depth(&parent)?;
    let config = config::load(&parent)?;
    let show = pick::show_messages(
        std::io::stdout().is_terminal(),
        std::io::stderr().is_terminal(),
        &args,
    );
    let dry_run = parent.get("CCX_DRY_RUN") == Some("1".as_ref());
    let self_exe = std::env::current_exe().ok();
    let claude = launch::find_claude(
        config.launch.claude_bin.as_deref(),
        &parent,
        self_exe.as_deref(),
    );
    let mut auto = None;
    let account = match launch::pick_account(&config, name, subcommand, &parent)? {
        Pick::Named(account) => account,
        Pick::Base(account) => {
            if let Some(word) = subcommand
                && show
            {
                eprintln!("{}", launch::base_note(word, account));
            }
            account
        }
        Pick::Auto => {
            let context = AutoContext {
                config: &config,
                env: &parent,
                claude: claude.as_deref(),
                depth,
                dry_run,
                allow_paid,
            };
            let (account, picked) = auto_select(&context, &mut args)?;
            auto = Some(picked);
            account
        }
    };
    // ccx asks before each launch on a paid API-key account (#20).
    let paid = account.is_paid();
    if paid && !auto.as_ref().is_some_and(|a| a.paid_consent) {
        paid::confirm_named(account, allow_paid)?;
    }
    let (mut env, mut removed) = launch::build_env(&parent, account, depth);
    if paid {
        // The key goes only into the environment of this one exec.
        paid::add_key(&mut env, account, &parent)?;
        removed.retain(|key| *key != paid::KEY_VAR);
    }

    // Banner and warnings only on a terminal and not for `-p`, so pipes
    // and scripts stay clean.
    if show {
        for key in &removed {
            eprintln!("ccx: warning: removed {key}; it overrides the account login");
        }
        if let Some(auto) = &auto {
            for line in &auto.notes {
                eprintln!("{line}");
            }
        }
        if config.launch.banner {
            match auto.as_ref().and_then(|a| a.banner.as_ref()) {
                Some(banner) => eprintln!("{banner}"),
                None if paid => eprintln!("{}", paid::banner(&account.name)),
                None => eprintln!("ccx: account {} ({})", account.name, account.dir.display()),
            }
        }
    }

    if dry_run {
        let report = launch::dry_run_report(account, claude.as_deref(), &parent, &env, &args);
        print!("{report}");
        return Ok(ExitCode::SUCCESS);
    }

    // The scheduled refresh refreshes an account more often after a launch.
    if let Ok(home) = config::home(&parent) {
        let _ = schedule::record_launch(home, &account.name, usage::now_ms());
    }

    if let Some((path, state)) = auto.as_ref().and_then(|a| a.state.as_ref())
        && let Err(e) = pick::write_state(path, state)
        && show
    {
        eprintln!("ccx: warning: {e}");
    }

    // Add missing links, for example a new skill. Never change a conflict.
    match sync::health_check(config.base(), account) {
        Ok(0) => {}
        Ok(n) if show => eprintln!("ccx: sync added {n} missing links in {}", account.name),
        Ok(_) => {}
        Err(e) if show => eprintln!("ccx: warning: sync check: {e}"),
        Err(_) => {}
    }

    let Some(claude) = claude else {
        eprintln!("ccx: cannot find claude; set launch.claude_bin or CCX_CLAUDE_BIN");
        return Ok(ExitCode::from(EXIT_EXEC_FAILED));
    };
    // Stale-while-revalidate: refresh the usage cache for the next launch.
    // The refresh outlives the exec. A failure never stops the launch.
    if auto.is_some()
        && !refresh::disabled(&parent)
        && let Some(exe) = &self_exe
        && let Err(e) = refresh::spawn_background(exe)
        && show
    {
        eprintln!("ccx: warning: cannot start the usage refresh: {e}");
    }
    let error = Command::new(&claude)
        .args(&args)
        .env_clear()
        .envs(env.iter())
        .exec();
    eprintln!("ccx: cannot run {}: {error}", claude.display());
    Ok(ExitCode::from(EXIT_EXEC_FAILED))
}

/// The inputs of an automatic pick.
struct AutoContext<'a> {
    config: &'a config::Config,
    env: &'a Env,
    claude: Option<&'a std::path::Path>,
    depth: u32,
    dry_run: bool,
    /// `--allow-paid`: use a paid fallback with no prompt.
    allow_paid: bool,
}

/// The result of an automatic pick: the messages and the state to write.
#[derive(Default)]
struct Auto {
    /// The usage banner of the account. `None` gives the plain account line.
    banner: Option<String>,
    /// Warnings and the handoff banner. ccx shows them on a terminal, also
    /// when `launch.banner` is false.
    notes: Vec<String>,
    /// The pick state to write, for a pick by usage.
    state: Option<(std::path::PathBuf, pick::State)>,
    /// True when the user agreed to the paid fallback (#20).
    paid_consent: bool,
}

/// The enabled accounts with their usage. When the usage data of an
/// account is `usage.fresh_minutes` old or more, or missing, ccx first runs
/// a blocking fetch for that account (#11).
fn fresh_candidates(cx: &AutoContext, home: &std::ffi::OsStr) -> Vec<pick::Candidate> {
    let config = cx.config;
    let candidates = pick::candidates(config, home);
    let Some(claude) = cx.claude.filter(|_| !refresh::disabled(cx.env)) else {
        return candidates;
    };
    let now = usage::now_ms();
    let stale: Vec<&config::Account> = candidates
        .iter()
        .filter(|c| {
            !c.paid
                && c.usage.as_ref().is_none_or(|u| {
                    !refresh::is_fresh(u.fetched_at_ms, now, config.usage.fresh_minutes)
                })
        })
        .filter_map(|c| config.account(&c.name))
        .collect();
    if stale.is_empty() {
        return candidates;
    }
    refresh::blocking(config, &stale, claude, cx.env, home, cx.depth);
    pick::candidates(config, home)
}

/// Automatic selection for a session with no account name. Only
/// `Pick::Auto` comes here, so claude subcommands never use the picker.
/// With no enabled account, ccx uses the base account.
///
/// For `-c` and `--resume <id>`, ccx first finds the owner of the session
/// (#15). Then `args` can change to `--resume <id>` for a handoff.
fn auto_select<'a>(
    cx: &AutoContext<'a>,
    args: &mut Vec<OsString>,
) -> config::Result<(&'a config::Account, Auto)> {
    let config = cx.config;
    let home = config::home(cx.env)?;
    let owner = resume::intent(args).and_then(|intent| {
        let enc = match intent.kind {
            resume::Kind::Continue => handoff::physical_cwd()
                .ok()
                .map(|cwd| handoff::encode_cwd(&cwd)),
            resume::Kind::Resume(_) => None,
        };
        resume::owner(config, &intent.kind, enc.as_deref()).map(|thread| (intent, thread))
    });
    let candidates = fresh_candidates(cx, home);
    let now = usage::now_ms();
    let state_path = pick::state_path(home);
    let last = pick::read_state(&state_path).last;
    let model = pick::model_arg(args);
    if let Some((intent, thread)) = owner {
        return auto_resume(
            cx,
            home,
            &candidates,
            &intent,
            &thread,
            args,
            last.as_deref(),
        );
    }
    let Some(decision) = pick::pick(
        &candidates,
        last.as_deref(),
        model.as_deref(),
        &config.pick,
        now,
    ) else {
        return Ok((config.base(), Auto::default()));
    };
    if let Some(index) = pick::paid_offer(&candidates, Some(&decision)) {
        let name = &candidates[index].name;
        let ask = || paid::ask(&paid::fallback_question(name));
        if paid::consent(cx.allow_paid, paid::terminal(), ask)? {
            let account = config
                .account(name)
                .expect("each candidate is a configured account");
            // No pick state: stickiness is only for subscription accounts.
            let auto = Auto {
                banner: Some(paid::banner(name)),
                paid_consent: true,
                ..Auto::default()
            };
            return Ok((account, auto));
        }
    }
    let chosen = &candidates[decision.index];
    let account = config
        .account(&chosen.name)
        .expect("each candidate is a configured account");
    let mut auto = Auto {
        banner: Some(pick::banner(chosen, now, pick::local_clock)),
        state: Some((state_path, chosen_state(&chosen.name, now))),
        ..Auto::default()
    };
    if let pick::Reason::AllLimited { frees_at_ms } = decision.reason {
        auto.notes.push(pick::all_limited_warning(
            &chosen.name,
            frees_at_ms,
            now,
            pick::local_clock,
        ));
    }
    Ok((account, auto))
}

fn chosen_state(name: &str, now: u64) -> pick::State {
    pick::State {
        last: Some(name.to_string()),
        picked_at_ms: Some(now),
    }
}

/// `-c` or `--resume <id>` for a session that `thread.account` owns:
/// launch the owner, or hand off to the account that automatic selection
/// picks when the owner is limited or is a paid API-key account (#15,
/// #20). When all subscription accounts are limited, ccx asks before it
/// uses a paid account, the same as for `ccx` with no arguments.
fn auto_resume<'a>(
    cx: &AutoContext<'a>,
    home: &std::ffi::OsStr,
    candidates: &[pick::Candidate],
    intent: &resume::Intent,
    thread: &handoff::Thread,
    args: &mut Vec<OsString>,
    last: Option<&str>,
) -> config::Result<(&'a config::Account, Auto)> {
    let config = cx.config;
    let owner = config
        .account(&thread.account)
        .expect("each thread is in a configured account");
    let owner_candidate = candidates
        .iter()
        .find(|c| c.name == owner.name)
        .cloned()
        .unwrap_or_else(|| pick::candidate(owner, home));
    let now = usage::now_ms();
    let model = pick::model_arg(args);
    let auto = Auto {
        banner: Some(pick::banner(&owner_candidate, now, pick::local_clock)),
        ..Auto::default()
    };
    let route = resume::route(
        candidates,
        &owner_candidate,
        last,
        model.as_deref(),
        &config.pick,
        now,
    );
    let hop = Hop {
        cx,
        home,
        owner,
        thread,
        intent,
        now,
    };
    let decision = match route {
        resume::Route::Owner => return Ok((owner, auto)),
        resume::Route::Handoff { index, reason } => {
            return Ok(hop.run(&candidates[index], &reason, auto, args));
        }
        resume::Route::AllLimited { decision } => decision,
    };

    // All subscription accounts are limited: offer a paid account. A paid
    // owner is the offer, so that the session does not move.
    let offer = if owner_candidate.paid {
        Some(&owner_candidate)
    } else {
        pick::paid_offer(candidates, decision.as_ref()).map(|i| &candidates[i])
    };
    if let Some(paid) = offer.filter(|_| decision.is_some()) {
        let ask = || paid::ask(&paid::fallback_question(&paid.name));
        if paid::consent(cx.allow_paid, paid::terminal(), ask)? {
            if paid.name == owner.name {
                let auto = Auto {
                    banner: Some(paid::banner(&owner.name)),
                    paid_consent: true,
                    ..Auto::default()
                };
                return Ok((owner, auto));
            }
            return Ok(hop.run(paid, "all subscription accounts limited", auto, args));
        }
    }
    match decision {
        // A paid owner with no yes: the soonest-reset subscription account.
        Some(decision) if owner_candidate.paid => {
            let chosen = &candidates[decision.index];
            let mut auto = auto;
            if let pick::Reason::AllLimited { frees_at_ms } = decision.reason {
                auto.notes.push(pick::all_limited_warning(
                    &chosen.name,
                    frees_at_ms,
                    now,
                    pick::local_clock,
                ));
            }
            Ok(hop.run(chosen, &resume::paid_reason(&owner.name), auto, args))
        }
        // A paid owner and no subscription account: `run_launch` asks
        // with the question for a named paid account.
        None if owner_candidate.paid => Ok((owner, auto)),
        _ => {
            let mut auto = auto;
            auto.notes.push(resume::all_limited_warning(&owner.name));
            Ok((owner, auto))
        }
    }
}

/// A handoff of `thread` from `owner` to a different account.
struct Hop<'a, 'b> {
    cx: &'b AutoContext<'a>,
    home: &'b std::ffi::OsStr,
    owner: &'a config::Account,
    thread: &'b handoff::Thread,
    intent: &'b resume::Intent,
    now: u64,
}

impl<'a> Hop<'a, '_> {
    /// Copies the session to `chosen` and gives the launch there. When the
    /// copy fails, the session stays in the owner. A paid target gets the
    /// paid banner and no pick state; the caller has its consent.
    fn run(
        &self,
        chosen: &pick::Candidate,
        reason: &str,
        mut auto: Auto,
        args: &mut Vec<OsString>,
    ) -> (&'a config::Account, Auto) {
        let config = self.cx.config;
        let (owner, thread, now) = (self.owner, self.thread, self.now);
        let target = config
            .account(&chosen.name)
            .expect("each candidate is a configured account");
        // A dry run copies nothing. It shows the launch for a new copy.
        let done = if self.cx.dry_run {
            Ok((thread.id.clone(), false))
        } else {
            let mut warnings = Vec::new();
            let done = handoff::handoff(
                self.home,
                owner,
                target,
                thread,
                SystemTime::now(),
                &mut |w| warnings.push(format!("ccx: warning: {w}")),
            );
            auto.notes.extend(warnings);
            done.map(|d| (d.id, d.plan == handoff::Plan::Fork))
        };
        match done {
            Ok((id, fork)) => {
                let fork = fork.then_some(id.as_str());
                auto.notes
                    .push(resume::banner(&owner.name, &target.name, reason, fork));
                if chosen.paid {
                    auto.banner = Some(paid::banner(&chosen.name));
                    auto.paid_consent = true;
                } else {
                    auto.banner = Some(pick::banner(chosen, now, pick::local_clock));
                    auto.state =
                        Some((pick::state_path(self.home), chosen_state(&chosen.name, now)));
                }
                *args = resume::target_args(args, self.intent, &id);
                (target, auto)
            }
            Err(e) => {
                auto.notes.push(format!(
                    "ccx: warning: cannot hand off to {}: {e}; the session stays in {}",
                    target.name, owner.name
                ));
                (owner, auto)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    /// The release archives include `completions/` (`dist-workspace.toml`).
    /// The files must agree with the CLI. To update them, run
    /// `CCX_UPDATE_GOLDEN=1 mise run test` and check the diff.
    #[test]
    fn completion_files_agree_with_the_cli() {
        use shell_init::Shell;
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("completions");
        let update = std::env::var_os("CCX_UPDATE_GOLDEN").is_some();
        for shell in [Shell::Zsh, Shell::Bash, Shell::Fish] {
            let path = dir.join(shell_init::completion_file_name(shell));
            let want = shell_init::completion_file(shell, Cli::command());
            if update {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(&path, &want).unwrap();
                continue;
            }
            let have = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(
                have == want,
                "{} differs from the CLI; run with CCX_UPDATE_GOLDEN=1 to update",
                path.display()
            );
        }
    }

    #[test]
    fn zsh_completion_file_is_an_autoload_file() {
        let text = shell_init::completion_file(shell_init::Shell::Zsh, Cli::command());
        assert!(text.starts_with("#compdef ccx\n"), "{text}");
        assert!(!text.contains("$+functions[compdef]"));
    }
}

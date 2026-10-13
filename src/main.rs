mod adopt;
mod config;
mod doctor;
mod env;
mod handoff;
mod launch;
mod list;
mod paths;
mod pick;
mod refresh;
mod service;
mod shell_init;
mod status;
mod sync;
mod usage;

use std::ffi::OsString;
use std::io::IsTerminal;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode};

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
    Adopt {
        /// Confirm the changes without a prompt.
        #[arg(short, long)]
        yes: bool,
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
    /// The service runs `ccx usage refresh --all --background` every 300
    /// seconds: launchd on macOS, a systemd user timer on Linux. With no
    /// systemd, `install` prints a crontab line and changes no files.
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
    /// Show if the job is installed and loaded, the last refresh and the
    /// age of each usage cache.
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
        } => run_launch(account.as_deref(), subcommand, args),
    }
}

fn run_own(args: Vec<OsString>) -> config::Result<ExitCode> {
    let cli = Cli::parse_from(std::iter::once(OsString::from("ccx")).chain(args));
    match cli.command {
        Own::Manage { command } => {
            let env = Env::from_process();
            match command {
                Manage::Adopt { yes } => adopt::run(&env, yes)?,
                Manage::List { json } => list::run(&env, json)?,
                Manage::ShellInit { shell } => shell_init::run(&env, shell, Cli::command())?,
                Manage::Sync {
                    name,
                    all: _,
                    dry_run,
                    force,
                } => {
                    if !sync::run(&env, name.as_deref(), dry_run, force)? {
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
                },
        } => {
            let ok = status::refresh(&Env::from_process(), name.as_deref(), background)?;
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
    args: Vec<OsString>,
) -> config::Result<ExitCode> {
    let parent = Env::from_process();
    let depth = launch::check_depth(&parent)?;
    let config = config::load(&parent)?;
    let show = pick::show_messages(
        std::io::stdout().is_terminal(),
        std::io::stderr().is_terminal(),
        &args,
    );
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
            let (account, picked) = auto_select(&config, &parent, &args, claude.as_deref(), depth)?;
            auto = picked;
            account
        }
    };
    let (env, removed) = launch::build_env(&parent, account, depth);

    // Banner and warnings only on a terminal and not for `-p`, so pipes
    // and scripts stay clean.
    if show {
        for key in &removed {
            eprintln!("ccx: warning: removed {key}; it overrides the account login");
        }
        if let Some(warning) = auto.as_ref().and_then(|a| a.warning.as_ref()) {
            eprintln!("{warning}");
        }
        if config.launch.banner {
            match &auto {
                Some(auto) => eprintln!("{}", auto.banner),
                None => eprintln!("ccx: account {} ({})", account.name, account.dir.display()),
            }
        }
    }

    if parent.get("CCX_DRY_RUN") == Some("1".as_ref()) {
        let report = launch::dry_run_report(account, claude.as_deref(), &parent, &env, &args);
        print!("{report}");
        return Ok(ExitCode::SUCCESS);
    }

    if let Some(auto) = &auto
        && let Err(e) = pick::write_state(&auto.state_path, &auto.state)
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

/// The result of an automatic pick: the messages and the state to write.
struct Auto {
    banner: String,
    warning: Option<String>,
    state_path: std::path::PathBuf,
    state: pick::State,
}

/// Automatic selection for a session with no account name. Only
/// `Pick::Auto` comes here, so claude subcommands never use the picker.
/// With no enabled account, ccx uses the base account.
///
/// When the usage data of an account is `usage.fresh_minutes` old or more,
/// or missing, ccx first runs a blocking fetch for that account (#11).
fn auto_select<'a>(
    config: &'a config::Config,
    env: &Env,
    args: &[OsString],
    claude: Option<&std::path::Path>,
    depth: u32,
) -> config::Result<(&'a config::Account, Option<Auto>)> {
    let home = config::home(env)?;
    let mut candidates = pick::candidates(config, home);
    if let Some(claude) = claude
        && !refresh::disabled(env)
    {
        let now = usage::now_ms();
        let stale: Vec<&config::Account> = candidates
            .iter()
            .filter(|c| {
                c.usage.as_ref().is_none_or(|u| {
                    !refresh::is_fresh(u.fetched_at_ms, now, config.usage.fresh_minutes)
                })
            })
            .filter_map(|c| config.account(&c.name))
            .collect();
        if !stale.is_empty() {
            refresh::blocking(config, &stale, claude, env, home, depth);
            candidates = pick::candidates(config, home);
        }
    }
    let now = usage::now_ms();
    let state_path = pick::state_path(home);
    let last = pick::read_state(&state_path).last;
    let model = pick::model_arg(args);
    let Some(decision) = pick::pick(
        &candidates,
        last.as_deref(),
        model.as_deref(),
        &config.pick,
        now,
    ) else {
        return Ok((config.base(), None));
    };
    let chosen = &candidates[decision.index];
    let account = config
        .account(&chosen.name)
        .expect("each candidate is a configured account");
    let warning = match decision.reason {
        pick::Reason::AllLimited { frees_at_ms } => Some(pick::all_limited_warning(
            &chosen.name,
            frees_at_ms,
            now,
            pick::local_clock,
        )),
        _ => None,
    };
    let auto = Auto {
        banner: pick::banner(chosen, now, pick::local_clock),
        warning,
        state_path,
        state: pick::State {
            last: Some(chosen.name.clone()),
            picked_at_ms: Some(now),
        },
    };
    Ok((account, Some(auto)))
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

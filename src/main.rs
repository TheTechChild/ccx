mod adopt;
mod config;
mod doctor;
mod env;
mod launch;
mod list;
mod paths;
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
    /// Move a session to a different account (not ready yet).
    Handoff(Rest),
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
    },
}

#[derive(Debug, clap::Args)]
struct Rest {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    args: Vec<String>,
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
    let word = match cli.command {
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
            }
            return Ok(ExitCode::SUCCESS);
        }
        Own::Status { json } => {
            return Ok(ExitCode::from(status::run(&Env::from_process(), json)?));
        }
        Own::Usage {
            command: UsageCommand::Refresh { name, all: _ },
        } => {
            let ok = status::refresh(&Env::from_process(), name.as_deref())?;
            return Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            });
        }
        Own::Handoff(_) => "handoff",
    };
    Err(format!("`ccx {word}` is not ready yet"))
}

fn run_launch(
    name: Option<&str>,
    subcommand: Option<&str>,
    args: Vec<OsString>,
) -> config::Result<ExitCode> {
    let parent = Env::from_process();
    let depth = launch::check_depth(&parent)?;
    let config = config::load(&parent)?;
    let account = match launch::pick_account(&config, name, subcommand, &parent)? {
        Pick::Named(account) => account,
        Pick::Base(account) => {
            if let Some(word) = subcommand
                && std::io::stderr().is_terminal()
            {
                eprintln!("{}", launch::base_note(word, account));
            }
            account
        }
        Pick::Auto => auto_select(&config),
    };
    let (env, removed) = launch::build_env(&parent, account, depth);
    let self_exe = std::env::current_exe().ok();
    let claude = launch::find_claude(
        config.launch.claude_bin.as_deref(),
        &parent,
        self_exe.as_deref(),
    );

    // Banner and warnings only on a terminal, so pipes stay clean.
    let tty = std::io::stdout().is_terminal() && std::io::stderr().is_terminal();
    if tty {
        for key in &removed {
            eprintln!("ccx: warning: removed {key}; it overrides the account login");
        }
        if config.launch.banner {
            eprintln!("ccx: account {} ({})", account.name, account.dir.display());
        }
    }

    if parent.get("CCX_DRY_RUN") == Some("1".as_ref()) {
        let report = launch::dry_run_report(account, claude.as_deref(), &parent, &env, &args);
        print!("{report}");
        return Ok(ExitCode::SUCCESS);
    }

    // Add missing links, for example a new skill. Never change a conflict.
    match sync::health_check(config.base(), account) {
        Ok(0) => {}
        Ok(n) if tty => eprintln!("ccx: sync added {n} missing links in {}", account.name),
        Ok(_) => {}
        Err(e) if tty => eprintln!("ccx: warning: sync check: {e}"),
        Err(_) => {}
    }

    let Some(claude) = claude else {
        eprintln!("ccx: cannot find claude; set launch.claude_bin or CCX_CLAUDE_BIN");
        return Ok(ExitCode::from(EXIT_EXEC_FAILED));
    };
    let error = Command::new(&claude)
        .args(&args)
        .env_clear()
        .envs(env.iter())
        .exec();
    eprintln!("ccx: cannot run {}: {error}", claude.display());
    Ok(ExitCode::from(EXIT_EXEC_FAILED))
}

/// Automatic selection for a session with no account name. Only
/// `Pick::Auto` comes here, so claude subcommands never fetch usage.
/// Automatic selection is not ready yet: use the base account.
fn auto_select(config: &config::Config) -> &config::Account {
    config.base()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}

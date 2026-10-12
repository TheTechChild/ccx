mod config;
mod env;
mod launch;
mod paths;

use std::ffi::OsString;
use std::io::IsTerminal;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode};

use clap::{Parser, Subcommand};

use crate::env::Env;
use crate::launch::Invocation;

/// The exit code when ccx cannot start claude.
const EXIT_EXEC_FAILED: u8 = 127;

/// Launch Claude Code on the least-used of several accounts.
///
/// `ccx [@name] [claude args...]` starts claude. ccx passes all arguments
/// after `@name` to claude. `ccx -- args...` passes all arguments.
#[derive(Debug, Parser)]
#[command(name = "ccx", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Own,
}

/// The commands that ccx keeps for itself (the reserved first words).
#[derive(Debug, Subcommand)]
enum Own {
    /// Manage accounts (not ready yet).
    Manage(Rest),
    /// Show the usage of each account (not ready yet).
    Status(Rest),
    /// Fetch and show usage (not ready yet).
    Usage(Rest),
    /// Move a session to a different account (not ready yet).
    Handoff(Rest),
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
        Invocation::Launch { account, args } => run_launch(account.as_deref(), args),
    }
}

fn run_own(args: Vec<OsString>) -> config::Result<ExitCode> {
    let cli = Cli::parse_from(std::iter::once(OsString::from("ccx")).chain(args));
    let word = match cli.command {
        Own::Manage(_) => "manage",
        Own::Status(_) => "status",
        Own::Usage(_) => "usage",
        Own::Handoff(_) => "handoff",
    };
    Err(format!("`ccx {word}` is not ready yet"))
}

fn run_launch(name: Option<&str>, args: Vec<OsString>) -> config::Result<ExitCode> {
    let parent = Env::from_process();
    let depth = launch::check_depth(&parent)?;
    let config = config::load(&parent)?;
    let account = launch::choose_account(&config, name, &parent)?;
    let (env, removed) = launch::build_env(&parent, account, depth);
    let self_exe = std::env::current_exe().ok();
    let claude = launch::find_claude(&config, &parent, self_exe.as_deref());

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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}

//! `ccx manage bootstrap`: set up ccx on a new machine in one command.
//!
//! The steps: find `claude`, get the base `~/.claude` from `[base].repo`,
//! sync all accounts, log in to the accounts that are not logged in, offer
//! the service, then show `ccx status` and the shell line. Bootstrap is
//! safe to run again: a step that is done already shows OK and changes
//! nothing.
//!
//! The logins go through `claude auth login` (ADR 0002). With no browser,
//! claude prints a login URL on the terminal. ccx does not capture or
//! change that output, and it adds no flag to `claude auth login`.

use std::ffi::OsStr;
use std::fs;
use std::io::{ErrorKind, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::add;
use crate::config::{self, Config, Result};
use crate::env::Env;
use crate::launch;
use crate::lifecycle;
use crate::service;
use crate::status;
use crate::sync;
use crate::wizard;

/// The native installer of Claude Code.
pub const INSTALL_COMMAND: &str = "curl -fsSL https://claude.ai/install.sh | bash";
/// The number of steps that bootstrap shows.
const STEPS: usize = 6;
/// The maximum number of files that a conflict message lists.
const MAX_LISTED: usize = 20;

/// The arguments of `ccx manage bootstrap`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Args {
    /// `Some(true)` for `--service`, `Some(false)` for `--no-service`, `None`
    /// to ask on a terminal.
    pub service: Option<bool>,
}

/// The result of one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// The step was done already. Nothing changed.
    Ok,
    /// The step changed something.
    Done,
    /// The step did not run.
    Skip,
    /// The step failed. Bootstrap continues and exits with code 1.
    Fail,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            State::Ok => "OK",
            State::Done => "DONE",
            State::Skip => "SKIP",
            State::Fail => "FAIL",
        }
    }
}

/// One line of the summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub name: &'static str,
    pub state: State,
    pub detail: String,
}

impl Step {
    fn new(name: &'static str, state: State, detail: impl Into<String>) -> Step {
        Step {
            name,
            state,
            detail: detail.into(),
        }
    }
}

/// `ccx manage bootstrap`. Returns false when a step failed.
pub fn run(parent: &Env, args: &Args) -> Result<bool> {
    let loaded = config::load(parent);
    let mut steps = Vec::new();

    header(1, "claude");
    let claude_bin = loaded
        .as_ref()
        .ok()
        .and_then(|c| c.launch.claude_bin.clone());
    let self_exe = std::env::current_exe().ok();
    let claude = launch::find_claude(claude_bin.as_deref(), parent, self_exe.as_deref())
        .filter(|path| launch::is_executable(path))
        .ok_or_else(missing_claude)?;
    println!("{}", claude.display());
    steps.push(Step::new("claude", State::Ok, claude.display().to_string()));

    let config = loaded.map_err(|e| missing_config(parent, e))?;
    let overlay = config::overlay_path(parent)?;
    if fs::symlink_metadata(&overlay).is_err() {
        eprintln!("{}", overlay_warning(&overlay));
    }

    header(2, "base account");
    let base = config.base();
    let step = base_step(parent, &base.dir, config.base_repo.as_deref())?;
    println!("{}", step.detail);
    steps.push(step);

    header(3, "sync");
    steps.push(sync_step(parent, &config)?);

    header(4, "login");
    steps.push(match lifecycle::login_missing(parent) {
        Ok(logins) if !logins.ok => Step::new("login", State::Fail, "a login failed"),
        Ok(logins) if logins.done == 0 => {
            Step::new("login", State::Ok, "all accounts are logged in")
        }
        Ok(logins) => Step::new("login", State::Done, logged_in(logins.done)),
        Err(e) => Step::new("login", State::Fail, e),
    });

    header(5, "service");
    let step = service_step(parent, args.service);
    println!("{}", step.detail);
    steps.push(step);

    header(6, "status");
    if let Err(e) = status::run(parent, false) {
        eprintln!("ccx: warning: {e}");
    }
    println!();
    println!("Add this line to your shell profile:");
    println!("  {}", shell_line(parent.get("SHELL")));

    println!();
    println!("Bootstrap summary:");
    print!("{}", render(&steps));
    Ok(steps.iter().all(|s| s.state != State::Fail))
}

fn header(n: usize, title: &str) {
    println!("[{n}/{STEPS}] {title}");
}

fn logged_in(n: usize) -> String {
    format!(
        "logged in to {n} {}",
        if n == 1 { "account" } else { "accounts" }
    )
}

/// The error when `claude` is not installed.
pub fn missing_claude() -> String {
    format!(
        "cannot find claude. Install Claude Code with the native installer, then run `ccx manage bootstrap` again:\n\n  {INSTALL_COMMAND}\n\nIf claude is in a different location, set launch.claude_bin or CCX_CLAUDE_BIN."
    )
}

fn missing_config(env: &Env, error: String) -> String {
    match config::config_path(env) {
        Ok(path) if fs::symlink_metadata(&path).is_err() => format!(
            "{error}. Link config.toml from your config repo to {}, or set CCX_CONFIG",
            path.display()
        ),
        _ => error,
    }
}

/// The warning when `config.local.toml` does not exist.
pub fn overlay_warning(path: &Path) -> String {
    format!(
        "ccx: warning: {} does not exist, so ccx does not check the email of each login. \
         To check the emails, stop now, add the file with an `email` for each account, and run bootstrap again.",
        path.display()
    )
}

/// The shell line for `$SHELL`. The default is zsh.
pub fn shell_line(shell: Option<&OsStr>) -> String {
    let name = shell
        .and_then(|s| Path::new(s).file_name())
        .and_then(OsStr::to_str)
        .unwrap_or("zsh");
    match name {
        "fish" => "ccx manage shell-init fish | source".to_string(),
        "bash" => "eval \"$(ccx manage shell-init bash)\"".to_string(),
        _ => "eval \"$(ccx manage shell-init zsh)\"".to_string(),
    }
}

/// The summary table.
pub fn render(steps: &[Step]) -> String {
    steps
        .iter()
        .map(|s| format!("  {:<5} {:<8} {}\n", s.state.label(), s.name, s.detail))
        .collect()
}

/// Step 2: gets the base dir from `repo`. An error stops bootstrap.
pub fn base_step(env: &Env, dir: &Path, repo: Option<&str>) -> Result<Step> {
    let shown = dir.display();
    let meta = match fs::symlink_metadata(dir) {
        Ok(meta) => Some(meta),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => return Err(format!("cannot read {shown}: {e}")),
    };
    if meta.is_some() && !dir.is_dir() {
        return Err(format!("{shown} is not a directory"));
    }
    let Some(repo) = repo else {
        if meta.is_some() {
            return Ok(Step::new(
                "base",
                State::Ok,
                format!("{shown} exists; the config has no [base].repo"),
            ));
        }
        add::make_dir(dir)?;
        return Ok(Step::new(
            "base",
            State::Done,
            format!("made {shown} (mode 0700); the config has no [base].repo"),
        ));
    };
    if meta.is_none() {
        git(env, None, &["clone", "-q", repo, &dir.to_string_lossy()])?;
        return Ok(Step::new(
            "base",
            State::Done,
            format!("cloned {repo} into {shown}"),
        ));
    }
    if fs::symlink_metadata(dir.join(".git")).is_ok() {
        return Ok(
            match git(env, Some(dir), &["remote", "get-url", "origin"]) {
                Ok(url) if url.trim() == repo => {
                    Step::new("base", State::Ok, format!("{shown} is a clone of {repo}"))
                }
                Ok(url) => Step::new(
                    "base",
                    State::Ok,
                    format!(
                        "{shown} is a git repo; warning: its origin is {}, not {repo}",
                        url.trim()
                    ),
                ),
                Err(_) => Step::new(
                    "base",
                    State::Ok,
                    format!("{shown} is a git repo; warning: it has no origin"),
                ),
            },
        );
    }
    convert(env, dir, repo)?;
    Ok(Step::new(
        "base",
        State::Done,
        format!("{shown} is now a clone of {repo}; the files that were in it did not change"),
    ))
}

/// Makes the existing dir `dir` a clone of `repo`: `git init`, `remote
/// add`, `fetch` and `checkout`. Before the checkout, it stops when a file
/// of the repo is in `dir` already. It then removes only the `.git` dir
/// that it made. It never changes or deletes a file that was in `dir`.
fn convert(env: &Env, dir: &Path, repo: &str) -> Result<()> {
    git(env, Some(dir), &["init", "-q"])?;
    let checked = (|| {
        git(env, Some(dir), &["remote", "add", "origin", repo])?;
        git(env, Some(dir), &["fetch", "-q", "origin"])?;
        git(env, Some(dir), &["remote", "set-head", "origin", "--auto"])?;
        let head = git(
            env,
            Some(dir),
            &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        )?;
        let head = head.trim().to_string();
        let tree = git(
            env,
            Some(dir),
            &["ls-tree", "-r", "-z", "--name-only", &head],
        )?;
        let paths: Vec<&str> = tree.split('\0').filter(|p| !p.is_empty()).collect();
        let found = conflicts(dir, &paths);
        if !found.is_empty() {
            return Err(conflict_message(dir, repo, &found));
        }
        Ok(head)
    })();
    let head = match checked {
        Ok(head) => head,
        Err(e) => {
            // Only the `.git` dir that `git init` made. The check above
            // found that it did not exist before.
            let _ = fs::remove_dir_all(dir.join(".git"));
            return Err(e);
        }
    };
    let branch = head.strip_prefix("origin/").unwrap_or(&head);
    git(
        env,
        Some(dir),
        &["checkout", "-q", "-b", branch, "--track", &head],
    )
    .map(drop)
}

/// The paths of `repo_paths` that a checkout into `dir` would overwrite: a
/// path that exists, or a parent of a path that exists and is not a
/// directory.
pub fn conflicts(dir: &Path, repo_paths: &[&str]) -> Vec<String> {
    let mut found = std::collections::BTreeSet::new();
    for path in repo_paths {
        let path = Path::new(path);
        if fs::symlink_metadata(dir.join(path)).is_ok() {
            found.insert(path.display().to_string());
            continue;
        }
        for parent in path.ancestors().skip(1) {
            if parent.as_os_str().is_empty() {
                continue;
            }
            if fs::symlink_metadata(dir.join(parent)).is_ok_and(|m| !m.is_dir()) {
                found.insert(parent.display().to_string());
            }
        }
    }
    found.into_iter().collect()
}

fn conflict_message(dir: &Path, repo: &str, found: &[String]) -> String {
    let mut list: Vec<String> = found
        .iter()
        .take(MAX_LISTED)
        .map(|p| format!("  {p}"))
        .collect();
    if found.len() > MAX_LISTED {
        list.push(format!("  and {} more", found.len() - MAX_LISTED));
    }
    format!(
        "{} is not a git repo, and it has files that are also in {repo}:\n{}\nccx does not overwrite files. Nothing changed. Move these files out of {}, then run `ccx manage bootstrap` again",
        dir.display(),
        list.join("\n"),
        dir.display()
    )
}

/// Runs `git ARGS` (in `dir` when given) and returns its stdout.
fn git(env: &Env, dir: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    if let Some(dir) = dir {
        command.arg("-C").arg(dir);
    }
    let out = command
        .args(args)
        .env_clear()
        .envs(env.iter())
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run git: {e}. Install git, then try again"))?;
    if !out.status.success() {
        return Err(format!(
            "`git {}` failed ({}): {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Step 3: makes the missing account dirs at mode 0700, then runs `ccx
/// manage sync --all`.
fn sync_step(env: &Env, config: &Config) -> Result<Step> {
    let mut made: Vec<PathBuf> = Vec::new();
    for account in config.accounts.iter().filter(|a| !a.is_base) {
        if fs::symlink_metadata(&account.dir).is_err() {
            add::make_dir(&account.dir)?;
            println!("Made {} (mode 0700)", account.dir.display());
            made.push(account.dir.clone());
        }
    }
    // On a terminal, sync asks which unknown items to share and asks to
    // move the CONFLICT items, as `ccx manage sync --all` does.
    let mut prompt = wizard::Terminal;
    let ask: Option<&mut dyn wizard::Prompt> = match terminal() {
        true => Some(&mut prompt),
        false => None,
    };
    let synced = sync::run_counted(env, None, false, false, ask)?;
    let changed = synced.changed + made.len();
    Ok(if !synced.ok {
        Step::new(
            "sync",
            State::Fail,
            "CONFLICT items did not change; run `ccx manage sync --all`",
        )
    } else if changed == 0 {
        Step::new("sync", State::Ok, "all links are in place")
    } else {
        Step::new("sync", State::Done, format!("{changed} changes"))
    })
}

/// Step 5: offers `ccx manage service install`.
fn service_step(env: &Env, wanted: Option<bool>) -> Step {
    let name = "service";
    match service::is_current(env) {
        Ok(true) => return Step::new(name, State::Ok, "the service is installed and loaded"),
        Ok(false) => {}
        Err(e) => return Step::new(name, State::Fail, e),
    }
    let install = match wanted {
        Some(install) => install,
        None if terminal() => match dialoguer::Confirm::new()
            .with_prompt("Install the background usage refresh (ccx manage service install)?")
            .default(false)
            .interact()
        {
            Ok(answer) => answer,
            Err(e) => return Step::new(name, State::Fail, format!("cannot read the answer: {e}")),
        },
        None => {
            return Step::new(
                name,
                State::Skip,
                "no terminal; to install, give --service or run `ccx manage service install`",
            );
        }
    };
    if !install {
        return Step::new(
            name,
            State::Skip,
            "not installed; to install, run `ccx manage service install`",
        );
    }
    match service::install(env) {
        Ok(()) if service::detect(env).is_ok_and(|k| k == service::Kind::Cron) => Step::new(
            name,
            State::Skip,
            "no launchd and no systemd; add the crontab line above",
        ),
        Ok(()) => Step::new(name, State::Done, "installed"),
        Err(e) => Step::new(name, State::Fail, e),
    }
}

fn terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_line_follows_the_shell() {
        let line = |s: &str| shell_line(Some(OsStr::new(s)));
        assert_eq!(line("/bin/zsh"), "eval \"$(ccx manage shell-init zsh)\"");
        assert_eq!(
            line("/usr/bin/bash"),
            "eval \"$(ccx manage shell-init bash)\""
        );
        assert_eq!(
            line("/opt/homebrew/bin/fish"),
            "ccx manage shell-init fish | source"
        );
        assert_eq!(shell_line(None), "eval \"$(ccx manage shell-init zsh)\"");
        assert_eq!(line("/bin/sh"), "eval \"$(ccx manage shell-init zsh)\"");
    }

    #[test]
    fn missing_claude_names_the_installer() {
        let text = missing_claude();
        assert!(text.contains(INSTALL_COMMAND), "{text}");
        assert!(text.contains("CCX_CLAUDE_BIN"), "{text}");
    }

    #[test]
    fn conflicts_finds_files_and_parents_that_are_files() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path();
        fs::write(dir.join("CLAUDE.md"), "mine").unwrap();
        fs::write(dir.join("agents"), "a file, not a dir").unwrap();
        fs::create_dir(dir.join("skills")).unwrap();
        fs::write(dir.join("history.jsonl"), "kept").unwrap();
        let found = conflicts(
            dir,
            &[
                "CLAUDE.md",
                "agents/a.md",
                "agents/b.md",
                "skills/x/SKILL.md",
            ],
        );
        assert_eq!(found, ["CLAUDE.md", "agents"]);
        assert!(conflicts(dir, &["settings.json", "skills/y.md"]).is_empty());
    }

    #[test]
    fn conflict_message_lists_at_most_twenty_files() {
        let found: Vec<String> = (0..25).map(|n| format!("f{n}")).collect();
        let text = conflict_message(Path::new("/h/.claude"), "/repo", &found);
        assert!(text.contains("  f19\n"), "{text}");
        assert!(!text.contains("  f20\n"), "{text}");
        assert!(text.contains("and 5 more"), "{text}");
        assert!(text.contains("Nothing changed"), "{text}");
    }

    #[test]
    fn render_aligns_the_states() {
        let text = render(&[
            Step::new("claude", State::Ok, "/bin/claude"),
            Step::new("service", State::Skip, "no terminal"),
        ]);
        assert_eq!(
            text,
            "  OK    claude   /bin/claude\n  SKIP  service  no terminal\n"
        );
    }
}

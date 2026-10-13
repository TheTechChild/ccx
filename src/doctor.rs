//! `ccx manage doctor`: check the ccx setup.
//!
//! Doctor only reads. It never changes a file. The link checks use the
//! sync planner, so doctor and `ccx manage sync` agree. Doctor never reads
//! a secret: the keychain check reads only the names of the items.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::config::{self, Account, Config, Result};
use crate::env::Env;
use crate::launch;
use crate::paths::{expand_tilde, tilde};
use crate::sync::{self, Action, Class, Plan, State, Want};

/// Variables that override the account. ccx removes or replaces them at
/// launch, but doctor tells the user.
pub const OVERRIDE_VARS: [&str; 5] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_SECURESTORAGE_CONFIG_DIR",
    "CLAUDE_CONFIG_DIR",
];

/// The keychain service name of the login of an account with
/// `CLAUDE_CONFIG_DIR` set. The base account uses the name with no suffix.
const KEYCHAIN_PREFIX: &str = "Claude Code-credentials";

/// The result of one check. The order is from good to bad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Ok => "OK",
            Level::Warn => "WARN",
            Level::Fail => "FAIL",
        }
    }

    /// 0 when all checks pass, 1 for a WARN and 2 for a FAIL.
    pub fn exit_code(self) -> u8 {
        self as u8
    }
}

/// One line of the doctor output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub level: Level,
    /// The name of the check, for example `link`.
    pub check: &'static str,
    pub message: String,
}

fn finding(level: Level, check: &'static str, message: impl Into<String>) -> Finding {
    Finding {
        level,
        check,
        message: message.into(),
    }
}

fn ok(check: &'static str, message: impl Into<String>) -> Finding {
    finding(Level::Ok, check, message)
}

fn warn(check: &'static str, message: impl Into<String>) -> Finding {
    finding(Level::Warn, check, message)
}

fn fail(check: &'static str, message: impl Into<String>) -> Finding {
    finding(Level::Fail, check, message)
}

/// The worst level of `findings`.
pub fn worst(findings: &[Finding]) -> Level {
    findings.iter().map(|f| f.level).max().unwrap_or(Level::Ok)
}

/// Runs `ccx manage doctor`. Returns the exit code.
pub fn run(env: &Env, as_json: bool) -> Result<u8> {
    let findings = check_all(env);
    if as_json {
        println!("{}", to_json(&findings));
    } else {
        print!("{}", render(&findings));
    }
    Ok(worst(&findings).exit_code())
}

/// Does all checks. A check that needs the config does not run when the
/// config does not load.
pub fn check_all(env: &Env) -> Vec<Finding> {
    let home = env.get("HOME").unwrap_or(OsStr::new(""));
    let mut out = Vec::new();
    let config = match config::load(env) {
        Ok(config) => {
            let path = config::config_path(env).map_or_else(|e| e, |p| tilde(&p, home));
            out.push(ok(
                "config",
                format!("{} accounts in {path}", config.accounts.len()),
            ));
            Some(config)
        }
        Err(e) => {
            out.push(fail("config", e));
            None
        }
    };
    out.push(check_claude(config.as_ref(), env, home));
    out.extend(check_env(env));
    let Some(config) = config else {
        return out;
    };

    out.extend(check_paths(&config, home));
    let base = config.base();
    if base.dir.is_dir() {
        out.push(check_stray(base, home));
        out.extend(check_unknown(base, home));
        out.extend(check_accounts(&config));
    }
    out.extend(check_home_paths(&config, home));
    if cfg!(target_os = "macos") {
        out.extend(check_keychain(&config, env));
    }
    out
}

/// `claude` on PATH and its version.
fn check_claude(config: Option<&Config>, env: &Env, home: &OsStr) -> Finding {
    let bin = config.and_then(|c| c.launch.claude_bin.as_deref());
    let self_exe = std::env::current_exe().ok();
    let Some(path) = launch::find_claude(bin, env, self_exe.as_deref()) else {
        return fail(
            "claude",
            "cannot find claude on PATH; install it or set launch.claude_bin",
        );
    };
    let shown = tilde(&path, home);
    if !launch::is_executable(&path) {
        return fail("claude", format!("{shown} is not an executable file"));
    }
    let output = Command::new(&path)
        .arg("--version")
        .env_clear()
        .envs(env.iter())
        .output();
    let version = match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string(),
        _ => return warn("claude", format!("{shown}: `claude --version` failed")),
    };
    let chosen = bin.is_some() || env.get("CCX_CLAUDE_BIN").is_some();
    let on_path = env
        .get("PATH")
        .is_some_and(|p| std::env::split_paths(p).any(|dir| Some(dir.as_path()) == path.parent()));
    if !chosen && !on_path {
        return warn(
            "claude",
            format!("claude is not on PATH; ccx uses {shown} ({version})"),
        );
    }
    ok("claude", format!("{shown} ({version})"))
}

/// The override variables that are set, also with an empty value. Doctor
/// never shows the values.
pub fn check_env(env: &Env) -> Vec<Finding> {
    let found: Vec<_> = OVERRIDE_VARS
        .iter()
        .filter(|key| env.contains(key))
        .map(|key| {
            let what = match *key {
                "CLAUDE_CONFIG_DIR" => "replaces",
                _ => "removes",
            };
            warn(
                "env",
                format!("{key} is set in this shell; ccx {what} it at launch"),
            )
        })
        .collect();
    if found.is_empty() {
        return vec![ok("env", "no override variables are set")];
    }
    found
}

/// Each account dir: the config text is canonical, and the dir exists.
fn check_paths(config: &Config, home: &OsStr) -> Vec<Finding> {
    let mut out = Vec::new();
    for account in &config.accounts {
        let name = &account.name;
        let shown = tilde(&account.dir, home);
        // Compare the text: `Path` equality ignores a trailing slash.
        if expand_tilde(&account.dir_text, home).as_os_str() != account.dir.as_os_str() {
            let want = match account.dir_text.starts_with('~') {
                true => shown.clone(),
                false => account.dir.display().to_string(),
            };
            out.push(warn(
                "path",
                format!(
                    "{name}: the dir `{}` is not canonical; write `{want}`",
                    account.dir_text
                ),
            ));
        }
        out.push(match fs::metadata(&account.dir) {
            Ok(meta) if meta.is_dir() => ok("path", format!("{name}: {shown}")),
            Ok(_) => fail("path", format!("{name}: {shown} is not a directory")),
            Err(_) => fail("path", format!("{name}: {shown} is missing")),
        });
    }
    out
}

/// The stray `~/.claude/.claude.json` of an earlier identity split.
fn check_stray(base: &Account, home: &OsStr) -> Finding {
    let path = base.dir.join(".claude.json");
    let shown = tilde(&path, home);
    if fs::symlink_metadata(&path).is_ok() {
        return warn(
            "stray",
            format!("{shown} is a stray file; `ccx manage adopt` moves it to a backup"),
        );
    }
    ok("stray", format!("no stray {shown}"))
}

/// Top-level base entries that are not on the share list or the
/// never-share list.
fn check_unknown(base: &Account, home: &OsStr) -> Vec<Finding> {
    let shown = tilde(&base.dir, home);
    let names = match sync::names(&base.dir) {
        Ok(names) => names,
        Err(e) => return vec![fail("unknown", e)],
    };
    let found: Vec<_> = names
        .iter()
        .map(|name| name.to_string_lossy())
        .filter(|name| sync::classify(name) == Class::Unknown)
        .map(|name| {
            warn(
                "unknown",
                format!("{name} in {shown} is not on the share list or the never-share list"),
            )
        })
        .collect();
    if found.is_empty() {
        return vec![ok("unknown", format!("no unknown entries in {shown}"))];
    }
    found
}

/// Link health and local-only skills of each account other than the base.
fn check_accounts(config: &Config) -> Vec<Finding> {
    let base = config.base();
    let base_skills: BTreeSet<OsString> = sync::names(&base.dir.join(sync::SKILLS))
        .unwrap_or_default()
        .into_iter()
        .collect();
    let mut links = Vec::new();
    let mut skills = Vec::new();
    for account in config.accounts.iter().filter(|a| !a.is_base) {
        if !account.dir.is_dir() {
            continue; // The path check shows a FAIL.
        }
        match sync::plan(&base.dir, &account.dir, &account.own) {
            Ok(plan) => links.extend(check_links(&account.name, &plan)),
            Err(e) => links.push(fail("link", format!("{}: {e}", account.name))),
        }
        match local_skills(account, &base_skills) {
            Ok(names) => skills.extend(names.into_iter().map(|skill| {
                warn(
                    "skill",
                    format!(
                        "local-only skill {skill} in {}; promote to base?",
                        account.name
                    ),
                )
            })),
            Err(e) => skills.push(fail("skill", format!("{}: {e}", account.name))),
        }
    }
    if skills.is_empty() {
        skills.push(ok("skill", "no local-only skills"));
    }
    links.extend(skills);
    links
}

/// The link health of one account from its sync plan. A shared item that is
/// a real file or directory is a FAIL: a write replaced the link, so the
/// account no longer gets changes from the base.
pub fn check_links(name: &str, plan: &Plan) -> Vec<Finding> {
    let skills_ok = plan
        .items
        .iter()
        .find(|item| item.path == Path::new(sync::SKILLS))
        .is_none_or(|item| matches!(item.action, Action::Ok | Action::Skip));
    let sync = format!("`ccx manage sync {name}`");
    let force = format!("`ccx manage sync {name} --force`");
    let mut out = Vec::new();
    for item in &plan.items {
        // A skills dir that is not correct gets one line, not one per skill.
        if item.path.parent() == Some(Path::new(sync::SKILLS)) && !skills_ok {
            continue;
        }
        let path = item.path.display();
        let kind = match item.state {
            State::Dir => "directory",
            _ => "file",
        };
        out.push(match (item.action, &item.want, &item.state) {
            (Action::Ok | Action::Skip, _, _) => continue,
            (Action::Conflict, Want::Link(_), _) => fail(
                "link",
                format!(
                    "{name}: {path} is a real {kind}, not a link to the base; \
                     {force} moves it to a backup and links it"
                ),
            ),
            (Action::Conflict, Want::Dir, _) => fail(
                "link",
                format!("{name}: {path} is a real file, not a directory; run {force}"),
            ),
            (Action::Relink, _, State::Link(old)) => warn(
                "link",
                format!(
                    "{name}: {path} links to {}, not to the base; run {sync}",
                    old.display()
                ),
            ),
            (Action::Mkdir, _, State::Link(_)) => warn(
                "link",
                format!("{name}: {path} is a link, not a real directory; run {sync}"),
            ),
            (Action::Mkdir, _, _) => warn("link", format!("{name}: {path} is missing; run {sync}")),
            _ => warn(
                "link",
                format!("{name}: the link {path} is missing; run {sync}"),
            ),
        });
    }
    if out.is_empty() {
        let n = plan.items.len();
        out.push(ok("link", format!("{name}: {n} shared items are correct")));
    }
    out
}

/// Real (not linked) children of `<account>/skills/` that sync does not
/// manage: not `synced`, not a dotfile, not a base skill (the link check
/// shows those) and not in `own`.
pub fn local_skills(account: &Account, base_skills: &BTreeSet<OsString>) -> Result<Vec<String>> {
    let dir = account.dir.join(sync::SKILLS);
    if sync::lstat(&dir)? != State::Dir {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for name in sync::names(&dir)? {
        let text = name.to_string_lossy().into_owned();
        let owned = account
            .own
            .iter()
            .any(|o| o == &format!("{}/{text}", sync::SKILLS));
        if text == sync::SKILLS_SYNCED || text.starts_with('.') || owned {
            continue;
        }
        if base_skills.contains(&name) {
            continue;
        }
        if !matches!(sync::lstat(&dir.join(&name))?, State::Link(_)) {
            out.push(text);
        }
    }
    Ok(out)
}

/// `hooks` and `statusLine` commands in each `settings.json` that use the
/// absolute home path. A linked `settings.json` is checked one time.
fn check_home_paths(config: &Config, home: &OsStr) -> Vec<Finding> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let mut accounts: Vec<_> = config.accounts.iter().collect();
    accounts.sort_by_key(|a| !a.is_base);
    for account in accounts {
        let path = account.dir.join("settings.json");
        let Ok(real) = fs::canonicalize(&path) else {
            continue;
        };
        if !seen.insert(real) {
            continue;
        }
        let shown = tilde(&path, home);
        let settings = match fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| serde_json::from_str(&text).map_err(|e| e.to_string()))
        {
            Ok(value) => value,
            Err(e) => {
                out.push(warn("home-path", format!("cannot read {shown}: {e}")));
                continue;
            }
        };
        for (key, command) in home_paths(&settings, &home.to_string_lossy()) {
            out.push(warn(
                "home-path",
                format!("{shown}: {key} uses the absolute home path; write `{command}`"),
            ));
        }
    }
    if out.is_empty() {
        out.push(ok("home-path", "no absolute home paths in settings.json"));
    }
    out
}

/// The `command` strings under `hooks` and `statusLine` that contain the
/// absolute home path. Returns the key path and the command with
/// `$HOME/...` in place of the home path.
pub fn home_paths(settings: &Value, home: &str) -> Vec<(String, String)> {
    fn walk(value: &Value, key: String, prefix: &str, out: &mut Vec<(String, String)>) {
        match value {
            Value::Object(map) => {
                for (name, child) in map {
                    let child_key = format!("{key}.{name}");
                    match child {
                        Value::String(command) if name == "command" => {
                            if command.contains(prefix) {
                                out.push((child_key, command.replace(prefix, "$HOME/")));
                            }
                        }
                        _ => walk(child, child_key, prefix, out),
                    }
                }
            }
            Value::Array(items) => {
                for (i, child) in items.iter().enumerate() {
                    walk(child, format!("{key}[{i}]"), prefix, out);
                }
            }
            _ => {}
        }
    }
    let home = home.trim_end_matches('/');
    let mut out = Vec::new();
    if home.is_empty() {
        return out;
    }
    let prefix = format!("{home}/");
    for key in ["hooks", "statusLine"] {
        if let Some(value) = settings.get(key) {
            walk(value, key.to_string(), &prefix, &mut out);
        }
    }
    out
}

/// Orphan keychain items: `Claude Code-credentials-*` names that match no
/// configured account dir. Only on macOS. `security dump-keychain` with no
/// `-d` prints the attributes, not the secret data.
fn check_keychain(config: &Config, env: &Env) -> Vec<Finding> {
    let output = Command::new("security")
        .arg("dump-keychain")
        .env_clear()
        .envs(env.iter())
        .output();
    let dump = match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        Ok(out) => {
            let error = String::from_utf8_lossy(&out.stderr);
            return vec![warn(
                "keychain",
                format!("cannot list the keychain item names: {}", error.trim()),
            )];
        }
        Err(e) => {
            return vec![warn(
                "keychain",
                format!("cannot list the keychain item names: {e}"),
            )];
        }
    };
    let found: Vec<_> = orphans(&services(&dump), &config.accounts)
        .into_iter()
        .map(|name| {
            warn(
                "keychain",
                format!("orphan keychain item `{name}` matches no account dir"),
            )
        })
        .collect();
    if found.is_empty() {
        return vec![ok("keychain", "no orphan keychain items")];
    }
    found
}

/// The keychain service name that Claude Code uses for the login of an
/// account with `CLAUDE_CONFIG_DIR=dir`: the first 8 hex digits of the
/// SHA-256 of the dir text.
pub fn service_name(dir: &Path) -> String {
    let digest = Sha256::digest(dir.as_os_str().as_encoded_bytes());
    let hex: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
    format!("{KEYCHAIN_PREFIX}-{hex}")
}

/// The `svce` (service) names in `security dump-keychain` output. Other
/// attributes are ignored.
pub fn services(dump: &str) -> BTreeSet<String> {
    dump.lines()
        .filter_map(|line| line.trim().strip_prefix("\"svce\"<blob>=\""))
        .filter_map(|rest| rest.strip_suffix('"'))
        .map(str::to_string)
        .collect()
}

/// The `Claude Code-credentials-*` services that no account other than the
/// base account uses. The base account uses the name with no suffix.
pub fn orphans(services: &BTreeSet<String>, accounts: &[Account]) -> Vec<String> {
    let known: BTreeSet<String> = accounts
        .iter()
        .filter(|a| !a.is_base)
        .map(|a| service_name(&a.dir))
        .collect();
    let prefix = format!("{KEYCHAIN_PREFIX}-");
    services
        .iter()
        .filter(|name| name.starts_with(&prefix) && !known.contains(*name))
        .cloned()
        .collect()
}

/// The findings as text: one line for each finding, then a summary.
pub fn render(findings: &[Finding]) -> String {
    let mut out = String::new();
    let mut counts = [0usize; 3];
    for f in findings {
        counts[f.level as usize] += 1;
        out.push_str(&format!(
            "{:<4}  {}: {}\n",
            f.level.label(),
            f.check,
            f.message
        ));
    }
    out.push_str(&format!(
        "\n{} OK, {} WARN, {} FAIL\n",
        counts[0], counts[1], counts[2]
    ));
    out
}

/// The findings as stable JSON.
pub fn to_json(findings: &[Finding]) -> String {
    let checks: Vec<_> = findings
        .iter()
        .map(|f| json!({ "check": f.check, "level": f.level.label(), "message": f.message }))
        .collect();
    let value = json!({ "checks": checks, "result": worst(findings).label() });
    serde_json::to_string_pretty(&value).expect("JSON values always serialize")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn account(name: &str, dir: &Path, is_base: bool) -> Account {
        Account {
            name: name.into(),
            dir: dir.to_path_buf(),
            dir_text: dir.display().to_string(),
            weight: None,
            aliases: Vec::new(),
            enabled: true,
            email: None,
            own: Vec::new(),
            is_base,
        }
    }

    #[test]
    fn exit_codes_follow_the_worst_level() {
        assert_eq!(worst(&[]).exit_code(), 0);
        assert_eq!(worst(&[ok("a", "x")]).exit_code(), 0);
        assert_eq!(worst(&[ok("a", "x"), warn("b", "y")]).exit_code(), 1);
        assert_eq!(
            worst(&[fail("c", "z"), warn("b", "y"), ok("a", "x")]).exit_code(),
            2
        );
    }

    #[test]
    fn env_reports_each_override_var_and_no_values() {
        let clean: Env = [("HOME", "/home/test"), ("PATH", "/bin")]
            .into_iter()
            .collect();
        assert_eq!(
            check_env(&clean),
            vec![ok("env", "no override variables are set")]
        );

        let mut env = clean.clone();
        for key in OVERRIDE_VARS {
            env.set(key, "secret-value");
        }
        let found = check_env(&env);
        assert_eq!(found.len(), OVERRIDE_VARS.len());
        for (f, key) in found.iter().zip(OVERRIDE_VARS) {
            assert_eq!(f.level, Level::Warn);
            assert!(f.message.starts_with(key), "{}", f.message);
            assert!(!f.message.contains("secret-value"));
        }
        assert!(found[4].message.contains("replaces"));

        // An empty value also overrides: launch removes it too.
        let empty: Env = [("ANTHROPIC_API_KEY", "")].into_iter().collect();
        assert_eq!(check_env(&empty)[0].level, Level::Warn);
    }

    #[test]
    fn link_health_uses_the_sync_plan() {
        let root = tempfile::tempdir().unwrap();
        let (base, acct) = (root.path().join("base"), root.path().join("acct"));
        fs::create_dir_all(base.join("skills/tdd")).unwrap();
        fs::create_dir(&acct).unwrap();
        for name in ["settings.json", "CLAUDE.md", "keybindings.json"] {
            fs::write(base.join(name), "{}").unwrap();
        }
        fs::create_dir(acct.join("skills")).unwrap();
        symlink(base.join("skills/tdd"), acct.join("skills/tdd")).unwrap();
        symlink(base.join("CLAUDE.md"), acct.join("CLAUDE.md")).unwrap();
        symlink("/elsewhere", acct.join("keybindings.json")).unwrap();
        // A write replaced the link with a real file.
        fs::write(acct.join("settings.json"), "{}").unwrap();

        let plan = sync::plan(&base, &acct, &[]).unwrap();
        let found = check_links("angel2", &plan);
        let lines: Vec<_> = found
            .iter()
            .map(|f| (f.level, f.message.split(';').next().unwrap()))
            .collect();
        assert_eq!(
            lines,
            vec![
                (
                    Level::Warn,
                    "angel2: keybindings.json links to /elsewhere, not to the base"
                ),
                (
                    Level::Fail,
                    "angel2: settings.json is a real file, not a link to the base"
                ),
            ]
        );
    }

    #[test]
    fn link_health_ok_and_skills_dir_problems() {
        let root = tempfile::tempdir().unwrap();
        let (base, acct) = (root.path().join("base"), root.path().join("acct"));
        fs::create_dir_all(base.join("skills/a")).unwrap();
        fs::create_dir_all(base.join("skills/b")).unwrap();
        fs::create_dir(&acct).unwrap();
        fs::write(base.join("CLAUDE.md"), "x").unwrap();
        symlink(base.join("CLAUDE.md"), acct.join("CLAUDE.md")).unwrap();

        // `skills` is missing: one line, not one line for each skill.
        let plan = sync::plan(&base, &acct, &[]).unwrap();
        let found = check_links("x", &plan);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].level, Level::Warn);
        assert!(found[0].message.starts_with("x: skills is missing"));

        fs::create_dir(acct.join("skills")).unwrap();
        symlink(base.join("skills/a"), acct.join("skills/a")).unwrap();
        symlink(base.join("skills/b"), acct.join("skills/b")).unwrap();
        let plan = sync::plan(&base, &acct, &[]).unwrap();
        assert_eq!(
            check_links("x", &plan),
            vec![ok("link", "x: 4 shared items are correct")]
        );
    }

    #[test]
    fn local_skills_are_real_children_that_sync_does_not_manage() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("acct");
        let skills = dir.join("skills");
        for name in ["mine", "synced", ".hidden", "base-skill", "kept"] {
            fs::create_dir_all(skills.join(name)).unwrap();
        }
        fs::write(skills.join("notes.md"), "x").unwrap();
        symlink("/somewhere", skills.join("linked")).unwrap();
        let mut acct = account("angel2", &dir, false);
        acct.own = vec!["skills/kept".into()];
        let base_skills = BTreeSet::from([OsString::from("base-skill")]);
        assert_eq!(
            local_skills(&acct, &base_skills).unwrap(),
            vec!["mine", "notes.md"]
        );

        // A `skills` link is never read.
        let other = root.path().join("other");
        fs::create_dir(&other).unwrap();
        symlink(&skills, other.join("skills")).unwrap();
        let acct = account("other", &other, false);
        assert!(local_skills(&acct, &base_skills).unwrap().is_empty());
    }

    #[test]
    fn home_paths_finds_hook_and_status_line_commands() {
        let settings = json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Bash",
                    "hooks": [
                        { "type": "command", "command": "/home/test/.claude/hooks/a.sh" },
                        { "type": "command", "command": "$HOME/.claude/hooks/b.sh" }
                    ]
                }],
                "Stop": [{ "hooks": [{ "type": "command", "command": "python3 /home/test/x.py" }] }]
            },
            "statusLine": { "type": "command", "command": "/home/test/.claude/statusline.py" },
            "env": { "command": "/home/test/not-checked" },
            "other": "/home/test/x"
        });
        assert_eq!(
            home_paths(&settings, "/home/test"),
            vec![
                (
                    "hooks.PreToolUse[0].hooks[0].command".to_string(),
                    "$HOME/.claude/hooks/a.sh".to_string()
                ),
                (
                    "hooks.Stop[0].hooks[0].command".to_string(),
                    "python3 $HOME/x.py".to_string()
                ),
                (
                    "statusLine.command".to_string(),
                    "$HOME/.claude/statusline.py".to_string()
                ),
            ]
        );
        // A path that only starts with the same text is not in HOME.
        assert!(home_paths(&settings, "/home/tes").is_empty());
        assert!(home_paths(&settings, "").is_empty());
    }

    #[test]
    fn service_name_is_the_sha256_prefix_of_the_dir() {
        // printf '%s' /Users/example/.claude-angel2 | shasum -a 256
        assert_eq!(
            service_name(Path::new("/Users/example/.claude-angel2")),
            "Claude Code-credentials-8810f3a3"
        );
    }

    #[test]
    fn keychain_orphans_use_names_only() {
        let dump = "\
keychain: \"/Users/example/Library/Keychains/login.keychain-db\"
class: \"genp\"
attributes:
    \"acct\"<blob>=\"user@example.com\"
    \"svce\"<blob>=\"Claude Code-credentials\"
attributes:
    \"svce\"<blob>=\"Claude Code-credentials-8810f3a3\"
attributes:
    \"svce\"<blob>=\"Claude Code-credentials-0badf00d\"
attributes:
    \"svce\"<blob>=\"Other app\"
    \"svce\"<blob>=<NULL>
";
        let names = services(dump);
        assert_eq!(names.len(), 4);
        let accounts = [
            account("main", Path::new("/Users/example/.claude"), true),
            account("angel2", Path::new("/Users/example/.claude-angel2"), false),
        ];
        assert_eq!(
            orphans(&names, &accounts),
            vec!["Claude Code-credentials-0badf00d"]
        );
    }

    #[test]
    fn text_and_json_output() {
        let findings = [ok("env", "clean"), fail("link", "x: bad")];
        assert_eq!(
            render(&findings),
            "OK    env: clean\nFAIL  link: x: bad\n\n1 OK, 0 WARN, 1 FAIL\n"
        );
        let value: Value = serde_json::from_str(&to_json(&findings)).unwrap();
        assert_eq!(value["result"], "FAIL");
        assert_eq!(value["checks"][1]["check"], "link");
        assert_eq!(value["checks"][1]["level"], "FAIL");
        assert_eq!(value["checks"][1]["message"], "x: bad");
    }
}

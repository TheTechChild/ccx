//! Copy the user MCP servers and the allowlisted preference keys of the
//! base `~/.claude.json` into other accounts (#17).
//!
//! Links cannot share these settings, because they are in the
//! `.claude.json` of each account. `ccx manage add` copies them one time
//! after the login. `ccx manage sync --prefs` copies them again.
//!
//! Safety rules:
//! - ccx adds an MCP server only through `claude mcp add-json -s user`.
//!   It never writes `mcpServers` and never overwrites or removes a server.
//! - ccx writes only the allowlisted keys, with an atomic write. All other
//!   keys stay the same.
//! - ccx changes nothing in an account that has a running session.
//! - ccx never reads MCP OAuth tokens. The re-auth list comes from the
//!   server config only. Output shows key names, never values.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Map, Value};

use crate::adopt;
use crate::config::{self, Account, Config, Result};
use crate::env::Env;
use crate::handoff;
use crate::launch;
use crate::sync;
use crate::usage::claude_json_path;

/// The user preferences that `/config` and the UI write. The onboarding
/// keys prevent a second onboarding flow in the new account.
pub const BUILT_IN: [&str; 11] = [
    "theme",
    "editorMode",
    "verbose",
    "autoCompactEnabled",
    "preferredNotifChannel",
    "autoUpdates",
    "diffSidebarOpen",
    "deepLinkTerminal",
    "claudeInChromeDefaultEnabled",
    "hasCompletedOnboarding",
    "lastOnboardingVersion",
];

/// Keys that ccx never copies, also when `[prefs] allow` names them:
/// identity, MCP servers, projects, usage stats and migration state.
const DENIED: [&str; 10] = [
    "oauthAccount",
    "userID",
    "machineID",
    "mcpServers",
    "projects",
    "numStartups",
    "firstStartTime",
    "skillUsage",
    "pluginUsage",
    "migrationVersion",
];
/// Key prefixes that ccx never copies: caches, tips and bridge OAuth state.
const DENIED_PREFIXES: [&str; 3] = ["cached", "tip", "bridgeOauth"];
/// Key suffixes that ccx never copies: caches, counters, usage stats and
/// migration state.
const DENIED_SUFFIXES: [&str; 6] = [
    "Cache",
    "CacheSlots",
    "Count",
    "Counts",
    "Usage",
    "MigrationComplete",
];

/// The mode of a new `.claude.json`.
const CLAUDE_JSON_MODE: u32 = 0o600;

/// True for a key that ccx never copies.
pub fn always_denied(key: &str) -> bool {
    DENIED.contains(&key)
        || DENIED_PREFIXES.iter().any(|p| key.starts_with(p))
        || DENIED_SUFFIXES.iter().any(|s| key.ends_with(s))
}

/// The allowlist: the built-in keys, plus `allow`, minus `deny`, minus the
/// always-denied keys. Also returns the always-denied keys that `allow`
/// names, for a warning.
pub fn allowlist(prefs: &config::Prefs) -> (Vec<String>, Vec<String>) {
    let mut keys: BTreeSet<String> = BUILT_IN.iter().map(|k| k.to_string()).collect();
    keys.extend(prefs.allow.iter().cloned());
    for key in &prefs.deny {
        keys.remove(key);
    }
    let refused = keys.iter().filter(|k| always_denied(k)).cloned().collect();
    keys.retain(|k| !always_denied(k));
    (keys.into_iter().collect(), refused)
}

/// What ccx does with one MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerAction {
    /// Run `claude mcp add-json -s user`.
    Add,
    /// The account has a server with this name. ccx does not change it.
    Skip,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Server {
    pub name: String,
    pub action: ServerAction,
    /// The config of the server in the base account.
    pub config: Value,
}

/// What ccx does with one preference key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// Write the base value.
    Set,
    /// The account has the same value.
    Ok,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub name: String,
    pub action: KeyAction,
}

/// The `mcpServers` table of a `.claude.json` object.
fn servers(json: &Map<String, Value>) -> Option<&Map<String, Value>> {
    json.get("mcpServers").and_then(Value::as_object)
}

/// The MCP server plan: ADD for each base server that is not in the
/// target, else SKIP.
pub fn plan_servers(base: &Map<String, Value>, target: &Map<String, Value>) -> Vec<Server> {
    let have = servers(target);
    servers(base)
        .into_iter()
        .flatten()
        .map(|(name, config)| Server {
            name: name.clone(),
            action: if have.is_some_and(|h| h.contains_key(name)) {
                ServerAction::Skip
            } else {
                ServerAction::Add
            },
            config: config.clone(),
        })
        .collect()
}

/// The key plan: one item for each allowlisted key that is in the base.
pub fn plan_keys(
    base: &Map<String, Value>,
    target: &Map<String, Value>,
    allow: &[String],
) -> Vec<Key> {
    allow
        .iter()
        .filter_map(|name| {
            let value = base.get(name)?;
            let action = if target.get(name) == Some(value) {
                KeyAction::Ok
            } else {
                KeyAction::Set
            };
            Some(Key {
                name: name.clone(),
                action,
            })
        })
        .collect()
}

/// Copies the SET keys from `base` into `target`. Other keys stay.
pub fn merge(target: &mut Map<String, Value>, base: &Map<String, Value>, keys: &[Key]) -> usize {
    let mut changed = 0;
    for key in keys.iter().filter(|k| k.action == KeyAction::Set) {
        if let Some(value) = base.get(&key.name) {
            target.insert(key.name.clone(), value.clone());
            changed += 1;
        }
    }
    changed
}

/// True when the user must log in to the server with `/mcp`: an `http` or
/// `sse` server with an `oauth` config, or with no static `Authorization`
/// header and no `headersHelper`. ccx examines only the server config.
pub fn needs_auth(config: &Value) -> bool {
    let kind = config
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("stdio");
    if !matches!(kind, "http" | "sse") {
        return false;
    }
    if config.get("oauth").is_some() {
        return true;
    }
    let static_auth = config
        .get("headers")
        .and_then(Value::as_object)
        .is_some_and(|h| h.keys().any(|k| k.eq_ignore_ascii_case("authorization")));
    !static_auth && config.get("headersHelper").is_none()
}

/// The plan text of one account. It shows names only, never values.
pub fn render(name: &str, servers: &[Server], keys: &[Key]) -> String {
    let mut out = format!("{name}:\n");
    for server in servers {
        let (label, note) = match server.action {
            ServerAction::Add => ("ADD", ""),
            ServerAction::Skip => ("SKIP", " (the account has it)"),
        };
        out.push_str(&format!("  {label:<6} mcp {}{note}\n", server.name));
    }
    for key in keys {
        let label = match key.action {
            KeyAction::Set => "SET",
            KeyAction::Ok => "OK",
        };
        out.push_str(&format!("  {label:<6} {}\n", key.name));
    }
    if servers.is_empty() && keys.is_empty() {
        out.push_str("  nothing to copy\n");
    }
    out
}

/// The re-auth line for the servers that ccx added.
pub fn reauth_line(names: &[&str]) -> String {
    if names.is_empty() {
        "MCP servers that need re-auth: none".to_string()
    } else {
        format!("MCP servers that need re-auth: {}", names.join(", "))
    }
}

/// Reads a `.claude.json` object. A missing file gives `None`.
pub fn read_object(path: &Path) -> Result<Option<Map<String, Value>>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    match serde_json::from_slice(&bytes) {
        Ok(Value::Object(map)) => Ok(Some(map)),
        Ok(_) => Err(format!("{} is not a JSON object", path.display())),
        Err(e) => Err(format!("cannot parse {}: {e}", path.display())),
    }
}

/// The pid of a running session of the account, from its
/// `sessions/<pid>.json` files.
pub fn running_session(account: &Account) -> Option<u32> {
    handoff::running_pid(&account.dir, |_| true)
}

/// Copies the base settings into accounts.
pub struct Copier<'a> {
    parent: &'a Env,
    home: &'a OsStr,
    claude: Option<PathBuf>,
    depth: u32,
    base: Map<String, Value>,
    allow: Vec<String>,
}

impl<'a> Copier<'a> {
    /// Reads the base `.claude.json`. Returns `None`, with a note, when it
    /// does not exist.
    pub fn new(parent: &'a Env, config: &Config, claude: Option<PathBuf>) -> Result<Option<Self>> {
        let home = config::home(parent)?;
        let depth = launch::check_depth(parent)?;
        let path = claude_json_path(config.base(), home);
        let Some(base) = read_object(&path)? else {
            println!("{} does not exist. Nothing to copy.", path.display());
            return Ok(None);
        };
        let (allow, refused) = allowlist(&config.prefs);
        for key in refused {
            eprintln!("ccx: warning: prefs.allow: ccx never copies `{key}`");
        }
        Ok(Some(Copier {
            parent,
            home,
            claude,
            depth,
            base,
            allow,
        }))
    }

    /// Copies the MCP servers and the preference keys into `account`.
    /// Returns false when ccx refused the account or an MCP server failed.
    pub fn copy(&self, account: &Account, dry_run: bool) -> Result<bool> {
        let name = &account.name;
        let path = claude_json_path(account, self.home);
        let target = read_object(&path)?.unwrap_or_default();
        let servers = plan_servers(&self.base, &target);
        let keys = plan_keys(&self.base, &target, &self.allow);
        print!("{}", render(name, &servers, &keys));
        let adds: Vec<&Server> = servers
            .iter()
            .filter(|s| s.action == ServerAction::Add)
            .collect();
        let auth: Vec<&str> = adds
            .iter()
            .filter(|s| needs_auth(&s.config))
            .map(|s| s.name.as_str())
            .collect();
        let changes = !adds.is_empty() || keys.iter().any(|k| k.action == KeyAction::Set);
        if changes && let Some(pid) = running_session(account) {
            println!(
                "{name}: a session is running (pid {pid}). ccx does not change {}. Stop the session, then run `ccx manage sync --prefs {name}`.",
                path.display()
            );
            return Ok(false);
        }
        if dry_run || !changes {
            if !adds.is_empty() {
                println!("{}", reauth_line(&auth));
            }
            return Ok(true);
        }

        let mut ok = true;
        for server in &adds {
            match self.add_server(account, server) {
                Ok(()) => println!("{name}: added MCP server {}", server.name),
                Err(e) => {
                    eprintln!("ccx: {name}: cannot add MCP server {}: {e}", server.name);
                    ok = false;
                }
            }
        }
        // `claude mcp add-json` writes the file. Read it again.
        let mut target = read_object(&path)?.unwrap_or_default();
        let keys = plan_keys(&self.base, &target, &self.allow);
        if keys.iter().any(|k| k.action == KeyAction::Set) {
            if let Some(pid) = running_session(account) {
                println!(
                    "{name}: a session is running (pid {pid}). ccx does not change {}.",
                    path.display()
                );
                return Ok(false);
            }
            let count = merge(&mut target, &self.base, &keys);
            let mode = fs::metadata(&path)
                .map(|m| m.permissions().mode() & 0o7777)
                .unwrap_or(CLAUDE_JSON_MODE);
            let mut text = serde_json::to_string_pretty(&Value::Object(target))
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            text.push('\n');
            let written = adopt::write_atomic(&path, &text, mode)?;
            println!(
                "{name}: wrote {count} preference keys to {}",
                written.display()
            );
        }
        if !adds.is_empty() {
            println!("{}", reauth_line(&auth));
            if !auth.is_empty() {
                println!("To log in to these servers, run `/mcp` in `ccx @{name}`.");
            }
        }
        Ok(ok)
    }

    /// Runs `claude mcp add-json -s user <name> <json>` in the account.
    fn add_server(&self, account: &Account, server: &Server) -> Result<()> {
        let claude = self
            .claude
            .as_deref()
            .ok_or("cannot find claude; set launch.claude_bin or CCX_CLAUDE_BIN")?;
        let json = serde_json::to_string(&server.config)
            .map_err(|e| format!("cannot encode the server config: {e}"))?;
        let (env, _) = launch::build_env(self.parent, account, self.depth);
        let out = Command::new(claude)
            .args(["mcp", "add-json", "-s", "user", &server.name, &json])
            .env_clear()
            .envs(env.iter())
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("cannot run {}: {e}", claude.display()))?;
        if out.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(format!(
            "`claude mcp add-json` failed ({}): {}",
            out.status,
            stderr.trim()
        ))
    }
}

/// Finds `claude` for the MCP server copy.
pub fn find_claude(config: &Config, env: &Env) -> Option<PathBuf> {
    let self_exe = std::env::current_exe().ok();
    launch::find_claude(
        config.launch.claude_bin.as_deref(),
        env,
        self_exe.as_deref(),
    )
}

/// Runs `ccx manage sync --prefs`. `name` is `None` for `--all`. Returns
/// false when ccx refused an account or a copy failed.
pub fn run(env: &Env, name: Option<&str>, dry_run: bool) -> Result<bool> {
    let config = config::load(env)?;
    let accounts = sync::targets(&config, name)?;
    if accounts.is_empty() {
        println!("No accounts to sync. Only the base account is in the config.");
        return Ok(true);
    }
    let Some(copier) = Copier::new(env, &config, find_claude(&config, env))? else {
        return Ok(true);
    };
    let mut ok = true;
    for account in accounts {
        match copier.copy(account, dry_run) {
            Ok(true) => {}
            Ok(false) => ok = false,
            Err(e) => {
                eprintln!("ccx: {}: {e}", account.name);
                ok = false;
            }
        }
    }
    if dry_run {
        println!("Dry run. Nothing changed.");
    }
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            _ => panic!("not an object"),
        }
    }

    fn prefs(allow: &[&str], deny: &[&str]) -> config::Prefs {
        config::Prefs {
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: deny.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn built_in_allowlist_is_the_list_of_the_issue() {
        let (allow, refused) = allowlist(&config::Prefs::default());
        let mut want: Vec<String> = BUILT_IN.iter().map(|k| k.to_string()).collect();
        want.sort();
        assert_eq!(allow, want);
        assert!(refused.is_empty());
        for key in BUILT_IN {
            assert!(!always_denied(key), "{key}");
        }
    }

    #[test]
    fn config_allow_adds_and_deny_removes_keys() {
        let (allow, _) = allowlist(&prefs(&["fooPref"], &["verbose", "theme"]));
        assert!(allow.contains(&"fooPref".to_string()));
        assert!(!allow.contains(&"verbose".to_string()));
        assert!(!allow.contains(&"theme".to_string()));
        assert!(allow.contains(&"editorMode".to_string()));
    }

    #[test]
    fn always_denied_keys_are_never_in_the_allowlist() {
        let names = [
            "oauthAccount",
            "userID",
            "machineID",
            "mcpServers",
            "projects",
            "cachedUsageUtilization",
            "modelAccessCache",
            "btwUseCount",
            "tipLifetimeShownCounts",
            "numStartups",
            "firstStartTime",
            "skillUsage",
            "pluginUsage",
            "tipsHistory",
            "migrationVersion",
            "opusProMigrationComplete",
            "bridgeOauthDeadExpiresAt",
            "clientDataCacheSlots",
        ];
        let (allow, refused) = allowlist(&prefs(&names, &[]));
        for name in names {
            assert!(always_denied(name), "{name}");
            assert!(!allow.contains(&name.to_string()), "{name}");
            assert!(refused.contains(&name.to_string()), "{name}");
        }
    }

    #[test]
    fn plan_keys_skips_keys_that_are_not_in_the_base() {
        let base = object(json!({"theme": "dark", "verbose": true, "userID": "x"}));
        let target = object(json!({"verbose": true, "editorMode": "vim"}));
        let (allow, _) = allowlist(&config::Prefs::default());
        let keys = plan_keys(&base, &target, &allow);
        assert_eq!(
            keys,
            [
                Key {
                    name: "theme".into(),
                    action: KeyAction::Set
                },
                Key {
                    name: "verbose".into(),
                    action: KeyAction::Ok
                },
            ]
        );
    }

    #[test]
    fn merge_changes_only_allowlisted_keys() {
        let base = object(json!({
            "theme": "dark",
            "autoUpdates": false,
            "oauthAccount": {"organizationUuid": "base-org"},
            "numStartups": 99,
            "mcpServers": {"a": {"command": "x"}},
        }));
        let mut target = object(json!({
            "theme": "light",
            "oauthAccount": {"organizationUuid": "target-org"},
            "numStartups": 1,
            "projects": {"/w": {"allowedTools": []}},
        }));
        let before = target.clone();
        let (allow, _) = allowlist(&config::Prefs::default());
        let keys = plan_keys(&base, &target, &allow);
        assert_eq!(merge(&mut target, &base, &keys), 2);
        assert_eq!(target["theme"], "dark");
        assert_eq!(target["autoUpdates"], false);
        for (key, value) in &before {
            if key != "theme" {
                assert_eq!(&target[key], value, "{key}");
            }
        }
        assert!(!target.contains_key("mcpServers"));
        assert_eq!(target.len(), before.len() + 1);
    }

    #[test]
    fn plan_servers_adds_missing_and_skips_existing_servers() {
        let base = object(json!({"mcpServers": {
            "a": {"command": "a"},
            "b": {"type": "http", "url": "https://b.example.com"},
        }}));
        let target = object(json!({"mcpServers": {"b": {"command": "mine"}}}));
        let plan = plan_servers(&base, &target);
        let got: Vec<(&str, ServerAction)> =
            plan.iter().map(|s| (s.name.as_str(), s.action)).collect();
        assert_eq!(got, [("a", ServerAction::Add), ("b", ServerAction::Skip)]);
        assert!(plan_servers(&object(json!({})), &target).is_empty());
        assert_eq!(plan_servers(&base, &object(json!({}))).len(), 2);
    }

    #[test]
    fn needs_auth_uses_the_server_config_only() {
        assert!(!needs_auth(&json!({"command": "npx", "args": []})));
        assert!(!needs_auth(&json!({"type": "stdio", "command": "x"})));
        assert!(needs_auth(
            &json!({"type": "http", "url": "https://x.example.com"})
        ));
        assert!(needs_auth(
            &json!({"type": "sse", "url": "https://x.example.com"})
        ));
        assert!(!needs_auth(&json!({
            "type": "http",
            "url": "https://x.example.com",
            "headers": {"authorization": "Bearer placeholder"}
        })));
        assert!(!needs_auth(&json!({
            "type": "http", "url": "https://x.example.com", "headersHelper": "/bin/h"
        })));
        assert!(needs_auth(&json!({
            "type": "http",
            "url": "https://x.example.com",
            "headers": {"Authorization": "Bearer placeholder"},
            "oauth": {"clientId": "c"}
        })));
    }

    #[test]
    fn render_shows_names_never_values() {
        let servers = vec![
            Server {
                name: "a".into(),
                action: ServerAction::Add,
                config: json!({"env": {"TOKEN": "secret-value"}}),
            },
            Server {
                name: "b".into(),
                action: ServerAction::Skip,
                config: json!({}),
            },
        ];
        let keys = vec![Key {
            name: "theme".into(),
            action: KeyAction::Set,
        }];
        let text = render("work", &servers, &keys);
        assert_eq!(
            text,
            "work:\n  ADD    mcp a\n  SKIP   mcp b (the account has it)\n  SET    theme\n"
        );
        assert!(!text.contains("secret-value"));
        assert_eq!(render("work", &[], &[]), "work:\n  nothing to copy\n");
        assert_eq!(reauth_line(&[]), "MCP servers that need re-auth: none");
        assert_eq!(
            reauth_line(&["a", "b"]),
            "MCP servers that need re-auth: a, b"
        );
    }

    #[test]
    fn copier_with_no_base_claude_json_copies_nothing() {
        let home = tempfile::tempdir().unwrap();
        let env: Env = [("HOME", home.path().as_os_str())].into_iter().collect();
        let config = config::parse(
            &config::Source {
                path: Path::new("config.toml"),
                text: "[accounts.main]\ndir = \"~/.claude\"\n",
            },
            None,
            home.path().as_os_str(),
        )
        .unwrap();
        assert!(Copier::new(&env, &config, None).unwrap().is_none());
        fs::write(home.path().join(".claude.json"), "{\"theme\": \"dark\"}").unwrap();
        let copier = Copier::new(&env, &config, None).unwrap().unwrap();
        assert_eq!(copier.base["theme"], "dark");
    }

    #[test]
    fn read_object_handles_missing_and_bad_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        assert_eq!(read_object(&path).unwrap(), None);
        fs::write(&path, "[]").unwrap();
        assert!(
            read_object(&path)
                .unwrap_err()
                .contains("not a JSON object")
        );
        fs::write(&path, "{").unwrap();
        assert!(read_object(&path).unwrap_err().contains("cannot parse"));
        fs::write(&path, "{\"theme\": \"dark\"}").unwrap();
        assert_eq!(read_object(&path).unwrap().unwrap()["theme"], "dark");
    }
}

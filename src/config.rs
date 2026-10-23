//! Load, merge and validate the ccx config.
//!
//! The schema is in issue #3 ("Config schema"). `config.toml` is public and
//! has no real emails. The optional overlay `config.local.toml` holds the
//! emails, and ccx merges it over `config.toml` table by table.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::env::Env;
use crate::paths::canonical;

pub type Result<T> = std::result::Result<T, String>;

/// The directory of the base account, before canonicalization.
pub const BASE_DIR: &str = "~/.claude";
/// The only email domain that `config.toml` can contain.
const PLACEHOLDER_DOMAIN: &str = "@example.com";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub launch: Launch,
    pub base_repo: Option<String>,
    /// All accounts, sorted by name.
    pub accounts: Vec<Account>,
    pub pick: Pick,
    pub usage: Usage,
    pub prefs: Prefs,
    /// Prices per model from `[pricing.<model>]` (#24). They override the
    /// built-in price table of `ccx manage spend`.
    pub pricing: BTreeMap<String, Price>,
    /// The `[sync]` lists of top-level base entries (#55).
    pub sync: SyncLists,
    /// Keys that ccx still reads but no longer uses, for example
    /// `accounts.work.weight` or `pick.stickiness`. Doctor shows them.
    pub retired: Vec<String>,
}

/// The `[sync]` table: unknown top-level entries of the base dir that the
/// user chose to share or not to share (#55).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncLists {
    /// Entries that sync links like a built-in shared item.
    pub share: Vec<String>,
    /// Entries that sync does not share and does not show as unknown.
    pub ignore: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    /// The canonical path of the `claude` binary, if the config sets one.
    pub claude_bin: Option<PathBuf>,
    pub banner: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub name: String,
    /// The canonical config directory.
    pub dir: PathBuf,
    /// The `dir` text as the config writes it, before canonicalization.
    pub dir_text: String,
    pub aliases: Vec<String>,
    pub enabled: bool,
    pub email: Option<String>,
    /// Shared items that this account keeps as its own files (#7), for
    /// example `settings.json` or `skills/tdd`. Sync shows them as SKIP.
    pub own: Vec<String>,
    /// True for the one account whose dir is `~/.claude`.
    pub is_base: bool,
    /// Where the key of an API-key account (`kind = "api_key"`) comes
    /// from. `None` for a subscription account. An API-key account is a
    /// paid fallback (#20).
    pub api_key: Option<KeySource>,
}

impl Account {
    /// True for an API-key account. ccx uses it only as a paid fallback.
    pub fn is_paid(&self) -> bool {
        self.api_key.is_some()
    }
}

/// Where ccx gets the key of an API-key account. The config never holds
/// the key itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    /// The OS secret store, item `ccx-<name>` (`key_store = "keyring"`).
    Keyring,
    /// The file `~/.config/ccx/keys/<name>` at mode 0600
    /// (`key_store = "file"`).
    File,
    /// The stdout of a shell command (`key_command = "..."`).
    Command(String),
}

/// Thresholds in percent used.
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub session_avoid: u32,
    pub weekly_avoid: u32,
    pub scoped_avoid: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Usage {
    pub fresh_minutes: u64,
    pub fetch_timeout_secs: u64,
    pub refresh_min_gap_secs: u64,
    /// Adapt the interval of the scheduled refresh to the usage (#22).
    pub adaptive: bool,
    /// The shortest interval of the scheduled refresh.
    pub min_interval_secs: u64,
    /// The longest interval of the scheduled refresh.
    pub max_interval_secs: u64,
}

/// Changes to the built-in allowlist of `.claude.json` keys (#17).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prefs {
    /// Keys to add to the allowlist.
    pub allow: Vec<String>,
    /// Keys to remove from the allowlist.
    pub deny: Vec<String>,
}

/// The price of one model in USD for each million tokens (#24).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,
    pub cache_read: f64,
}

impl Config {
    pub fn account(&self, name: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.name == name)
    }

    pub fn base(&self) -> &Account {
        self.accounts
            .iter()
            .find(|a| a.is_base)
            .expect("validation makes sure that one base account exists")
    }

    pub fn names(&self) -> Vec<&str> {
        self.accounts.iter().map(|a| a.name.as_str()).collect()
    }
}

// The file format. Every key is optional here, so that the same types can
// check `config.toml`, the overlay and the merged result.

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    launch: Option<RawLaunch>,
    base: Option<RawBase>,
    #[serde(default)]
    accounts: BTreeMap<String, RawAccount>,
    pick: Option<RawPick>,
    usage: Option<RawUsage>,
    prefs: Option<RawPrefs>,
    #[serde(default)]
    pricing: BTreeMap<String, RawPrice>,
    sync: Option<RawSync>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSync {
    share: Option<Vec<String>>,
    ignore: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPrice {
    input: Option<f64>,
    output: Option<f64>,
    cache_write: Option<f64>,
    cache_read: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLaunch {
    claude_bin: Option<String>,
    banner: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBase {
    repo: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAccount {
    dir: Option<String>,
    /// Retired: ccx fills accounts in order and ignores the weight.
    weight: Option<toml::Value>,
    aliases: Option<Vec<String>>,
    enabled: Option<bool>,
    email: Option<String>,
    own: Option<Vec<String>>,
    kind: Option<String>,
    key_store: Option<String>,
    key_command: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPick {
    session_avoid: Option<u32>,
    weekly_avoid: Option<u32>,
    scoped_avoid: Option<u32>,
    /// Retired: ccx fills accounts in order and ignores it.
    stickiness: Option<toml::Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawUsage {
    fresh_minutes: Option<u64>,
    fetch_timeout_secs: Option<u64>,
    refresh_min_gap_secs: Option<u64>,
    adaptive: Option<bool>,
    min_interval_secs: Option<u64>,
    max_interval_secs: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPrefs {
    allow: Option<Vec<String>>,
    deny: Option<Vec<String>>,
}

/// The path of `config.toml`: `$CCX_CONFIG`, else
/// `$XDG_CONFIG_HOME/ccx/config.toml`, else `~/.config/ccx/config.toml`.
pub fn config_path(env: &Env) -> Result<PathBuf> {
    if let Some(path) = env.get("CCX_CONFIG") {
        return Ok(PathBuf::from(path));
    }
    Ok(config_dir(env)?.join("config.toml"))
}

/// The path of the overlay. It never depends on `$CCX_CONFIG` or on the
/// target of a `config.toml` symlink, so the overlay never lands in a repo.
pub fn overlay_path(env: &Env) -> Result<PathBuf> {
    Ok(config_dir(env)?.join("config.local.toml"))
}

/// The ccx config dir: `$XDG_CONFIG_HOME/ccx`, else `~/.config/ccx`.
pub fn config_dir(env: &Env) -> Result<PathBuf> {
    if let Some(xdg) = env.get("XDG_CONFIG_HOME").map(Path::new)
        && xdg.is_absolute()
    {
        return Ok(xdg.join("ccx"));
    }
    Ok(Path::new(home(env)?).join(".config/ccx"))
}

pub fn home(env: &Env) -> Result<&OsStr> {
    env.get("HOME").ok_or_else(|| "HOME is not set".to_string())
}

/// Reads and validates `config.toml` and the optional overlay.
pub fn load(env: &Env) -> Result<Config> {
    let main_path = config_path(env)?;
    let main_text = std::fs::read_to_string(&main_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!(
                "cannot read config {}: the file does not exist; run `ccx manage adopt` to make it, or `ccx manage bootstrap` to set up a new machine",
                main_path.display()
            )
        } else {
            format!("cannot read config {}: {e}", main_path.display())
        }
    })?;
    let overlay_path = overlay_path(env)?;
    let overlay_text = match std::fs::read_to_string(&overlay_path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("cannot read {}: {e}", overlay_path.display())),
    };
    let main = Source {
        path: &main_path,
        text: &main_text,
    };
    let overlay = overlay_text.as_deref().map(|text| Source {
        path: &overlay_path,
        text,
    });
    parse(&main, overlay.as_ref(), home(env)?)
}

/// The text of one config file and the path that error messages show.
pub struct Source<'a> {
    pub path: &'a Path,
    pub text: &'a str,
}

/// Merges the overlay over `config.toml` and validates the result.
pub fn parse(main: &Source, overlay: Option<&Source>, home: &OsStr) -> Result<Config> {
    let (mut table, main_raw) = parse_file(main)?;
    check_emails(main, &main_raw)?;

    if let Some(overlay) = overlay {
        let (overlay_table, overlay_raw) = parse_file(overlay)?;
        for name in overlay_raw.accounts.keys() {
            if !main_raw.accounts.contains_key(name) {
                return Err(format!(
                    "{}: account `{name}` is not in {}",
                    overlay.path.display(),
                    main.path.display()
                ));
            }
        }
        merge(&mut table, overlay_table);
    }

    let merged: RawConfig = toml::Value::Table(table)
        .try_into()
        .map_err(|e| format!("{}: {e}", main.path.display()))?;
    build(merged, home).map_err(|e| format!("{}: {e}", main.path.display()))
}

fn parse_file(source: &Source) -> Result<(toml::Table, RawConfig)> {
    let error = |e: toml::de::Error| format!("{}: {e}", source.path.display());
    let raw: RawConfig = toml::from_str(source.text).map_err(error)?;
    let table: toml::Table = toml::from_str(source.text).map_err(error)?;
    Ok((table, raw))
}

/// In `config.toml`, an email is an error unless it is a placeholder.
fn check_emails(main: &Source, raw: &RawConfig) -> Result<()> {
    for (name, account) in &raw.accounts {
        if let Some(email) = &account.email
            && !email.to_ascii_lowercase().ends_with(PLACEHOLDER_DOMAIN)
        {
            return Err(format!(
                "{}: accounts.{name}.email: put real emails only in config.local.toml",
                main.path.display()
            ));
        }
    }
    Ok(())
}

/// Merges `overlay` into `base`. Tables merge key by key. Other values in
/// the overlay replace the value in `base`.
fn merge(base: &mut toml::Table, overlay: toml::Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(old)), toml::Value::Table(new)) => merge(old, new),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

fn build(raw: RawConfig, home: &OsStr) -> Result<Config> {
    let base_dir = canonical(BASE_DIR, home);
    let sync = sync_lists(raw.sync.unwrap_or_default())?;
    let mut accounts = Vec::new();
    let mut dirs = BTreeSet::new();
    let mut retired = Vec::new();
    for (name, account) in raw.accounts {
        if !valid_name(&name) {
            return Err(format!(
                "accounts.{name}: the name must match ^[a-z0-9][a-z0-9_-]*$"
            ));
        }
        let dir = account
            .dir
            .clone()
            .ok_or_else(|| format!("accounts.{name}: missing key `dir`"))?;
        let dir_text = dir;
        let dir = canonical(&dir_text, home);
        if !dir.is_absolute() {
            return Err(format!(
                "accounts.{name}.dir: `{}` is not an absolute path",
                dir.display()
            ));
        }
        if !dirs.insert(dir.clone()) {
            return Err(format!(
                "accounts.{name}.dir: another account uses `{}`",
                dir.display()
            ));
        }
        if account.weight.is_some() {
            retired.push(format!("accounts.{name}.weight"));
        }
        let api_key = key_source(&name, &account)?;
        if api_key.is_some() && dir == base_dir {
            return Err(format!(
                "accounts.{name}.kind: the base account cannot be an API-key account"
            ));
        }
        let own = account.own.unwrap_or_default();
        if let Some(item) = own
            .iter()
            .find(|item| !crate::sync::valid_own(item, &sync.share))
        {
            return Err(format!(
                "accounts.{name}.own: `{item}` is not a shared item or `skills/<name>`"
            ));
        }
        accounts.push(Account {
            is_base: dir == base_dir,
            name,
            dir,
            dir_text,
            aliases: account.aliases.unwrap_or_default(),
            enabled: account.enabled.unwrap_or(true),
            email: account.email,
            own,
            api_key,
        });
    }
    // Dirs are unique, so at most one account is the base account.
    if !accounts.iter().any(|a| a.is_base) {
        return Err(format!(
            "no account has dir `{BASE_DIR}` (the base account)"
        ));
    }

    let pick = raw.pick.unwrap_or_default();
    if pick.stickiness.is_some() {
        retired.push("pick.stickiness".into());
    }
    let pick = Pick {
        session_avoid: percent("pick.session_avoid", pick.session_avoid, 90)?,
        weekly_avoid: percent("pick.weekly_avoid", pick.weekly_avoid, 95)?,
        scoped_avoid: percent("pick.scoped_avoid", pick.scoped_avoid, 98)?,
    };
    let usage = raw.usage.unwrap_or_default();
    let usage = Usage {
        fresh_minutes: usage.fresh_minutes.unwrap_or(15),
        fetch_timeout_secs: usage.fetch_timeout_secs.unwrap_or(5),
        refresh_min_gap_secs: usage.refresh_min_gap_secs.unwrap_or(60),
        adaptive: usage.adaptive.unwrap_or(true),
        min_interval_secs: usage.min_interval_secs.unwrap_or(120),
        max_interval_secs: usage.max_interval_secs.unwrap_or(1800),
    };
    if usage.min_interval_secs == 0 {
        return Err("usage.min_interval_secs: must be 1 or more".into());
    }
    if usage.min_interval_secs > usage.max_interval_secs {
        return Err(format!(
            "usage.min_interval_secs ({}) must be less than or equal to usage.max_interval_secs ({})",
            usage.min_interval_secs, usage.max_interval_secs
        ));
    }
    let mut pricing = BTreeMap::new();
    for (model, raw_price) in raw.pricing {
        let value = |key: &str, value: Option<f64>| match value {
            Some(v) if v.is_finite() && v >= 0.0 => Ok(v),
            Some(_) => Err(format!(
                "pricing.{model}.{key}: must be a number that is 0 or more"
            )),
            None => Err(format!("pricing.{model}: missing key `{key}`")),
        };
        let price = Price {
            input: value("input", raw_price.input)?,
            output: value("output", raw_price.output)?,
            cache_write: value("cache_write", raw_price.cache_write)?,
            cache_read: value("cache_read", raw_price.cache_read)?,
        };
        pricing.insert(model, price);
    }
    let prefs = raw.prefs.unwrap_or_default();
    let launch = raw.launch.unwrap_or_default();
    Ok(Config {
        launch: Launch {
            claude_bin: launch.claude_bin.map(|bin| canonical(&bin, home)),
            banner: launch.banner.unwrap_or(true),
        },
        base_repo: raw.base.and_then(|b| b.repo),
        accounts,
        pick,
        usage,
        prefs: Prefs {
            allow: prefs.allow.unwrap_or_default(),
            deny: prefs.deny.unwrap_or_default(),
        },
        pricing,
        sync,
        retired,
    })
}

/// Validates the `[sync]` lists. Each name is a top-level entry that is
/// not in a built-in group, and no name is in the two lists.
fn sync_lists(raw: RawSync) -> Result<SyncLists> {
    let lists = SyncLists {
        share: raw.share.unwrap_or_default(),
        ignore: raw.ignore.unwrap_or_default(),
    };
    for (key, names) in [("share", &lists.share), ("ignore", &lists.ignore)] {
        for name in names {
            crate::sync::check_list_name(name)
                .map_err(|why| format!("sync.{key}: `{name}` {why}"))?;
        }
    }
    if let Some(name) = lists.share.iter().find(|n| lists.ignore.contains(n)) {
        return Err(format!(
            "sync.share and sync.ignore: `{name}` is in the two lists; remove it from one list"
        ));
    }
    Ok(lists)
}

/// The key source of an account from `kind`, `key_store` and
/// `key_command`. `None` for a subscription account.
fn key_source(name: &str, account: &RawAccount) -> Result<Option<KeySource>> {
    let store = account.key_store.as_deref();
    let command = account.key_command.as_deref();
    match account.kind.as_deref() {
        None | Some("subscription") => {
            if store.is_some() || command.is_some() {
                let key = if store.is_some() {
                    "key_store"
                } else {
                    "key_command"
                };
                return Err(format!(
                    "accounts.{name}.{key}: only an account with kind = \"api_key\" can have it"
                ));
            }
            Ok(None)
        }
        Some("api_key") => match (store, command) {
            (Some("keyring"), None) => Ok(Some(KeySource::Keyring)),
            (Some("file"), None) => Ok(Some(KeySource::File)),
            (Some(other), None) => Err(format!(
                "accounts.{name}.key_store: `{other}` is not \"keyring\" or \"file\""
            )),
            (None, Some(command)) if !command.trim().is_empty() => {
                Ok(Some(KeySource::Command(command.to_string())))
            }
            (None, Some(_)) => Err(format!("accounts.{name}.key_command: is empty")),
            (Some(_), Some(_)) => Err(format!(
                "accounts.{name}: set key_store or key_command, not both"
            )),
            (None, None) => Err(format!(
                "accounts.{name}: an API-key account needs key_store or key_command"
            )),
        },
        Some(other) => Err(format!(
            "accounts.{name}.kind: `{other}` is not \"subscription\" or \"api_key\""
        )),
    }
}

fn percent(key: &str, value: Option<u32>, default: u32) -> Result<u32> {
    let value = value.unwrap_or(default);
    if value > 100 {
        return Err(format!("{key}: must be from 0 to 100"));
    }
    Ok(value)
}

/// Names match `^[a-z0-9][a-z0-9_-]*$`.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    let lower_or_digit = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    chars.next().is_some_and(lower_or_digit)
        && chars.all(|c| lower_or_digit(c) || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/test";

    const MINIMAL: &str = r#"
        [accounts.main]
        dir = "~/.claude"

        [accounts.angel2]
        dir = "~/.claude-angel2"
    "#;

    fn env(pairs: &[(&str, &str)]) -> Env {
        pairs.iter().copied().collect()
    }

    fn parse_with(main: &str, overlay: Option<&str>) -> Result<Config> {
        let main = Source {
            path: Path::new("config.toml"),
            text: main,
        };
        let overlay = overlay.map(|text| Source {
            path: Path::new("config.local.toml"),
            text,
        });
        parse(&main, overlay.as_ref(), OsStr::new(HOME))
    }

    fn parse_ok(main: &str) -> Config {
        parse_with(main, None).expect("config is valid")
    }

    fn parse_err(main: &str) -> String {
        parse_with(main, None).expect_err("config is not valid")
    }

    // Lookup order.

    #[test]
    fn config_path_prefers_ccx_config() {
        let e = env(&[
            ("CCX_CONFIG", "/x/my.toml"),
            ("XDG_CONFIG_HOME", "/xdg"),
            ("HOME", HOME),
        ]);
        assert_eq!(config_path(&e).unwrap(), PathBuf::from("/x/my.toml"));
    }

    #[test]
    fn config_path_uses_xdg_config_home_next() {
        let e = env(&[("XDG_CONFIG_HOME", "/xdg"), ("HOME", HOME)]);
        assert_eq!(
            config_path(&e).unwrap(),
            PathBuf::from("/xdg/ccx/config.toml")
        );
    }

    #[test]
    fn config_path_falls_back_to_dot_config() {
        let e = env(&[("HOME", HOME)]);
        let want = PathBuf::from("/home/test/.config/ccx/config.toml");
        assert_eq!(config_path(&e).unwrap(), want);
        // Empty and relative XDG values are ignored.
        let e = env(&[("XDG_CONFIG_HOME", ""), ("CCX_CONFIG", ""), ("HOME", HOME)]);
        assert_eq!(config_path(&e).unwrap(), want);
        let e = env(&[("XDG_CONFIG_HOME", "rel"), ("HOME", HOME)]);
        assert_eq!(config_path(&e).unwrap(), want);
    }

    #[test]
    fn config_path_needs_home_without_other_settings() {
        assert!(config_path(&env(&[])).is_err());
    }

    #[test]
    fn overlay_path_uses_xdg_then_dot_config_never_ccx_config() {
        let e = env(&[
            ("CCX_CONFIG", "/repo/config.toml"),
            ("XDG_CONFIG_HOME", "/xdg"),
            ("HOME", HOME),
        ]);
        assert_eq!(
            overlay_path(&e).unwrap(),
            PathBuf::from("/xdg/ccx/config.local.toml")
        );
        let e = env(&[("CCX_CONFIG", "/repo/config.toml"), ("HOME", HOME)]);
        assert_eq!(
            overlay_path(&e).unwrap(),
            PathBuf::from("/home/test/.config/ccx/config.local.toml")
        );
    }

    #[test]
    fn load_reads_both_files_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path().join(".config/ccx");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("config.toml"), MINIMAL).unwrap();
        std::fs::write(
            config_dir.join("config.local.toml"),
            "[accounts.angel2]\nemail = \"work@example.com\"\n",
        )
        .unwrap();
        let e: Env = [("HOME", dir.path().as_os_str())].into_iter().collect();
        let config = load(&e).unwrap();
        let angel2 = config.account("angel2").unwrap();
        assert_eq!(angel2.email.as_deref(), Some("work@example.com"));
        assert_eq!(angel2.dir, dir.path().join(".claude-angel2"));
    }

    #[test]
    fn load_reports_a_missing_config_with_its_path() {
        let e = env(&[("HOME", "/nonexistent-home")]);
        let err = load(&e).unwrap_err();
        assert!(
            err.contains("/nonexistent-home/.config/ccx/config.toml"),
            "{err}"
        );
    }

    // Accounts and defaults.

    #[test]
    fn minimal_config_loads_with_defaults() {
        let config = parse_ok(MINIMAL);
        assert_eq!(config.names(), ["angel2", "main"]);
        assert_eq!(config.base().name, "main");
        let angel2 = config.account("angel2").unwrap();
        assert_eq!(angel2.dir, PathBuf::from("/home/test/.claude-angel2"));
        assert!(!angel2.is_base);
        assert!(angel2.enabled);
        assert!(angel2.aliases.is_empty());
        assert!(config.retired.is_empty());
        assert!(config.launch.banner);
        assert_eq!(config.launch.claude_bin, None);
        assert_eq!(config.base_repo, None);
        assert_eq!(config.prefs, Prefs::default());
    }

    #[test]
    fn empty_pick_and_usage_use_defaults() {
        let config = parse_ok(&format!("{MINIMAL}\n[pick]\n[usage]\n"));
        let pick = config.pick;
        let usage = config.usage;
        assert_eq!(
            [pick.session_avoid, pick.weekly_avoid, pick.scoped_avoid,],
            [90, 95, 98]
        );
        assert_eq!(
            [
                usage.fresh_minutes,
                usage.fetch_timeout_secs,
                usage.refresh_min_gap_secs
            ],
            [15, 5, 60]
        );
        assert!(usage.adaptive);
        assert_eq!(
            [usage.min_interval_secs, usage.max_interval_secs],
            [120, 1800]
        );
    }

    #[test]
    fn interval_keys_load_and_min_is_at_most_max() {
        let config = parse_ok(&format!(
            "{MINIMAL}\n[usage]\nadaptive = false\nmin_interval_secs = 300\nmax_interval_secs = 300\n"
        ));
        assert!(!config.usage.adaptive);
        assert_eq!(config.usage.min_interval_secs, 300);
        assert_eq!(config.usage.max_interval_secs, 300);

        let err = parse_err(&format!(
            "{MINIMAL}\n[usage]\nmin_interval_secs = 600\nmax_interval_secs = 300\n"
        ));
        assert!(
            err.contains("usage.min_interval_secs (600) must be less than or equal to usage.max_interval_secs (300)"),
            "{err}"
        );
        // The default max (1800) applies when only min is given.
        let err = parse_err(&format!("{MINIMAL}\n[usage]\nmin_interval_secs = 1801\n"));
        assert!(err.contains("(1800)"), "{err}");
        let err = parse_err(&format!("{MINIMAL}\n[usage]\nmin_interval_secs = 0\n"));
        assert!(
            err.contains("usage.min_interval_secs: must be 1 or more"),
            "{err}"
        );
        let err = parse_err(&format!("{MINIMAL}\n[usage]\nadaptive = \"yes\"\n"));
        assert!(err.contains("adaptive"), "{err}");
    }

    #[test]
    fn full_schema_loads() {
        let config = parse_ok(
            r#"
            [launch]
            claude_bin = "~/bin/claude"
            banner = false

            [base]
            repo = "https://github.com/example/claude-config.git"

            [accounts.main]
            dir = "~/.claude"

            [accounts.angel2]
            dir = "~/.claude-angel2"
            aliases = ["claude2"]
            enabled = false

            [pick]
            session_avoid = 80
            weekly_avoid = 0
            scoped_avoid = 100

            [usage]
            fresh_minutes = 1
            fetch_timeout_secs = 2
            refresh_min_gap_secs = 3

            [prefs]
            allow = ["fooPref"]
            deny = ["verbose"]
            "#,
        );
        assert_eq!(config.prefs.allow, ["fooPref"]);
        assert_eq!(config.prefs.deny, ["verbose"]);
        assert_eq!(
            config.launch.claude_bin,
            Some(PathBuf::from("/home/test/bin/claude"))
        );
        assert!(!config.launch.banner);
        assert_eq!(
            config.base_repo.as_deref(),
            Some("https://github.com/example/claude-config.git")
        );
        let angel2 = config.account("angel2").unwrap();
        assert_eq!(angel2.aliases, ["claude2"]);
        assert!(!angel2.enabled);
        assert_eq!(config.pick.session_avoid, 80);
        assert_eq!(config.pick.scoped_avoid, 100);
        assert_eq!(config.usage.refresh_min_gap_secs, 3);
    }

    #[test]
    fn sync_lists_load_and_default_to_empty() {
        assert_eq!(parse_ok(MINIMAL).sync, SyncLists::default());
        let config = parse_ok(&format!(
            "{MINIMAL}\n[sync]\nshare = [\"notes\"]\nignore = [\"gh-pr-status-cache.json\"]\n"
        ));
        assert_eq!(config.sync.share, ["notes"]);
        assert_eq!(config.sync.ignore, ["gh-pr-status-cache.json"]);
    }

    #[test]
    fn sync_list_names_are_unknown_top_level_entries() {
        for (key, name, why) in [
            ("share", "", "is not a top-level entry name"),
            ("share", "a/b", "is not a top-level entry name"),
            ("ignore", ".", "is not a top-level entry name"),
            ("ignore", "..", "is not a top-level entry name"),
            ("share", ".claude.json", "is on the never-share list"),
            ("ignore", "projects", "is on the never-share list"),
            ("share", "settings.json", "is a built-in shared item"),
            ("ignore", "skills", "is a built-in shared item"),
        ] {
            let e = parse_err(&format!("{MINIMAL}\n[sync]\n{key} = [\"{name}\"]\n"));
            assert!(e.contains(&format!("sync.{key}: `{name}` {why}")), "{e}");
        }
        let e = parse_err(&format!(
            "{MINIMAL}\n[sync]\nshare = [\"a\", \"b\"]\nignore = [\"b\"]\n"
        ));
        assert!(
            e.contains("sync.share and sync.ignore: `b` is in the two lists"),
            "{e}"
        );
        let e = parse_err(&format!("{MINIMAL}\n[sync]\nother = []\n"));
        assert!(e.contains("other"), "{e}");
    }

    #[test]
    fn own_can_name_a_share_item() {
        let text = "\n[accounts.work]\ndir = \"~/.claude-work\"\nown = [\"notes\"]\n";
        let e = parse_err(&format!("{MINIMAL}{text}"));
        assert!(e.contains("accounts.work.own: `notes`"), "{e}");
        let config = parse_ok(&format!("{MINIMAL}{text}\n[sync]\nshare = [\"notes\"]\n"));
        assert_eq!(config.account("work").unwrap().own, ["notes"]);
    }

    #[test]
    fn own_loads_and_defaults_to_empty() {
        let config = parse_ok(&format!(
            "{MINIMAL}\n[accounts.work]\ndir = \"~/.claude-work\"\nown = [\"settings.json\", \"skills/tdd\"]\n"
        ));
        assert_eq!(
            config.account("work").unwrap().own,
            ["settings.json", "skills/tdd"]
        );
        assert!(config.account("angel2").unwrap().own.is_empty());
    }

    #[test]
    fn own_must_name_a_shared_item() {
        for bad in ["projects", "skills/synced", "chrome"] {
            let err = parse_err(&format!(
                "{MINIMAL}\n[accounts.work]\ndir = \"~/.claude-work\"\nown = [\"{bad}\"]\n"
            ));
            assert!(err.contains("accounts.work.own"), "{err}");
            assert!(err.contains(bad), "{err}");
        }
    }

    // API-key accounts (#20).

    #[test]
    fn api_key_accounts_load_with_each_key_source() {
        let config = parse_ok(&format!(
            r#"{MINIMAL}
            [accounts.paid]
            dir = "~/.claude-paid"
            kind = "api_key"
            key_store = "keyring"
            [accounts.paidfile]
            dir = "~/.claude-paidfile"
            kind = "api_key"
            key_store = "file"
            [accounts.paidcmd]
            dir = "~/.claude-paidcmd"
            kind = "api_key"
            key_command = "pass show anthropic"
            [accounts.sub]
            dir = "~/.claude-sub"
            kind = "subscription"
            "#
        ));
        let source = |name: &str| config.account(name).unwrap().api_key.clone();
        assert_eq!(source("paid"), Some(KeySource::Keyring));
        assert_eq!(source("paidfile"), Some(KeySource::File));
        assert_eq!(
            source("paidcmd"),
            Some(KeySource::Command("pass show anthropic".into()))
        );
        assert_eq!(source("sub"), None);
        assert_eq!(source("angel2"), None);
        assert!(config.account("paid").unwrap().is_paid());
        assert!(!config.account("angel2").unwrap().is_paid());
    }

    #[test]
    fn api_key_settings_that_are_not_valid_are_errors() {
        let cases = [
            ("kind = \"api_key\"", "needs key_store or key_command"),
            (
                "kind = \"api_key\"\nkey_store = \"file\"\nkey_command = \"x\"",
                "not both",
            ),
            (
                "kind = \"api_key\"\nkey_store = \"vault\"",
                "key_store: `vault`",
            ),
            (
                "kind = \"api_key\"\nkey_command = \" \"",
                "key_command: is empty",
            ),
            ("key_store = \"keyring\"", "only an account with kind"),
            (
                "kind = \"subscription\"\nkey_command = \"x\"",
                "key_command",
            ),
            ("kind = \"team\"", "kind: `team`"),
        ];
        for (keys, want) in cases {
            let err = parse_err(&format!(
                "{MINIMAL}\n[accounts.paid]\ndir = \"~/.claude-paid\"\n{keys}\n"
            ));
            assert!(err.contains("accounts.paid"), "{keys}: {err}");
            assert!(err.contains(want), "{keys}: {err}");
        }
    }

    #[test]
    fn base_account_cannot_be_an_api_key_account() {
        let err = parse_err(
            "[accounts.main]\ndir = \"~/.claude\"\nkind = \"api_key\"\nkey_store = \"keyring\"\n",
        );
        assert!(
            err.contains("base account cannot be an API-key account"),
            "{err}"
        );
    }

    // Validation rules.

    #[test]
    fn zero_base_accounts_is_an_error() {
        let err = parse_err("[accounts.a]\ndir = \"~/.claude-a\"\n");
        assert!(err.contains("no account has dir `~/.claude`"), "{err}");
    }

    #[test]
    fn two_base_accounts_is_an_error() {
        // Both dirs are `~/.claude` after canonicalization.
        let err = parse_err(
            "[accounts.a]\ndir = \"~/.claude\"\n[accounts.b]\ndir = \"/home/test/.claude/\"\n",
        );
        assert!(err.contains("another account uses"), "{err}");
    }

    #[test]
    fn base_account_matches_after_canonicalization() {
        let config = parse_ok("[accounts.home]\ndir = \"/home/test/./.claude/\"\n");
        assert_eq!(config.base().name, "home");
    }

    #[test]
    fn duplicate_dirs_are_an_error() {
        let err = parse_err(&format!(
            "{MINIMAL}\n[accounts.other]\ndir = \"~/.claude-angel2/\"\n"
        ));
        assert!(
            err.contains("accounts.other.dir: another account uses"),
            "{err}"
        );
    }

    #[test]
    fn relative_dir_is_an_error() {
        let err = parse_err(&format!(
            "{MINIMAL}\n[accounts.rel]\ndir = \"claude-rel\"\n"
        ));
        assert!(
            err.contains("accounts.rel.dir: `claude-rel` is not an absolute path"),
            "{err}"
        );
    }

    #[test]
    fn missing_dir_is_an_error() {
        let err = parse_err(&format!("{MINIMAL}\n[accounts.nodir]\nenabled = true\n"));
        assert!(err.contains("accounts.nodir: missing key `dir`"), "{err}");
    }

    #[test]
    fn names_must_match_the_pattern() {
        for good in ["a", "0", "angel2", "work_2", "a-b"] {
            assert!(valid_name(good), "{good}");
        }
        for bad in ["", "Angel", "-a", "_a", "a.b", "a b", "ä"] {
            assert!(!valid_name(bad), "{bad}");
        }
        let err = parse_err(&format!("{MINIMAL}\n[accounts.Work]\ndir = \"~/.w\"\n"));
        assert!(err.contains("accounts.Work: the name must match"), "{err}");
    }

    #[test]
    fn retired_keys_load_and_are_listed() {
        // Old configs still load. Any value is accepted, because ccx
        // ignores it.
        let config = parse_ok(
            "[accounts.main]\ndir = \"~/.claude\"\nweight = 20\n\
             [accounts.work]\ndir = \"~/.claude-work\"\nweight = -1\n\
             [pick]\nstickiness = 500\n",
        );
        assert_eq!(
            config.retired,
            [
                "accounts.main.weight",
                "accounts.work.weight",
                "pick.stickiness"
            ]
        );
    }

    #[test]
    fn pick_values_must_be_from_0_to_100() {
        for key in ["session_avoid", "weekly_avoid", "scoped_avoid"] {
            let err = parse_err(&format!("{MINIMAL}\n[pick]\n{key} = 101\n"));
            assert!(
                err.contains(&format!("pick.{key}: must be from 0 to 100")),
                "{err}"
            );
            let err = parse_err(&format!("{MINIMAL}\n[pick]\n{key} = -1\n"));
            assert!(err.contains(key), "{err}");
        }
    }

    #[test]
    fn unknown_key_names_the_file_and_the_key() {
        let err = parse_err(&format!("{MINIMAL}\n[launch]\ncolour = true\n"));
        assert!(err.starts_with("config.toml:"), "{err}");
        assert!(err.contains("colour"), "{err}");

        let err = parse_err(&format!(
            "{MINIMAL}\n[accounts.x]\ndir = \"~/.x\"\nmine = 1\n"
        ));
        assert!(err.contains("mine"), "{err}");

        let err = parse_err(&format!("{MINIMAL}\n[surprise]\n"));
        assert!(err.contains("surprise"), "{err}");

        let err = parse_with(MINIMAL, Some("[usage]\nfresh = 1\n")).unwrap_err();
        assert!(err.starts_with("config.local.toml:"), "{err}");
        assert!(err.contains("fresh"), "{err}");
    }

    #[test]
    fn wrong_type_is_an_error_with_the_file() {
        let err = parse_err(&format!("{MINIMAL}\n[launch]\nbanner = \"yes\"\n"));
        assert!(err.starts_with("config.toml:"), "{err}");
    }

    // Emails.

    #[test]
    fn real_looking_email_in_config_toml_is_an_error() {
        let email = ["someone", "example.org"].join("@");
        let err = parse_err(&format!(
            "[accounts.main]\ndir = \"~/.claude\"\nemail = \"{email}\"\n"
        ));
        assert!(err.contains("accounts.main.email"), "{err}");
        assert!(err.contains("config.local.toml"), "{err}");
        // The error does not repeat the email.
        assert!(!err.contains(&email), "{err}");
    }

    #[test]
    fn placeholder_email_in_config_toml_loads() {
        let config =
            parse_ok("[accounts.main]\ndir = \"~/.claude\"\nemail = \"main@example.com\"\n");
        assert_eq!(config.base().email.as_deref(), Some("main@example.com"));
    }

    // Overlay.

    #[test]
    fn missing_overlay_is_ok() {
        let config = parse_with(MINIMAL, None).unwrap();
        assert_eq!(config.base().email, None);
    }

    #[test]
    fn overlay_values_replace_config_values() {
        let main = format!("{MINIMAL}\n[pick]\nsession_avoid = 50\nweekly_avoid = 60\n");
        let overlay = r#"
            [accounts.main]
            email = "main@example.com"
            [accounts.angel2]
            enabled = false
            [pick]
            session_avoid = 70
        "#;
        let config = parse_with(&main, Some(overlay)).unwrap();
        assert_eq!(config.base().email.as_deref(), Some("main@example.com"));
        let angel2 = config.account("angel2").unwrap();
        assert!(!angel2.enabled);
        // Keys that the overlay does not set stay.
        assert_eq!(angel2.dir, PathBuf::from("/home/test/.claude-angel2"));
        assert_eq!(config.pick.session_avoid, 70);
        assert_eq!(config.pick.weekly_avoid, 60);
    }

    #[test]
    fn overlay_can_hold_a_real_looking_email() {
        let email = ["someone", "example.org"].join("@");
        let overlay = format!("[accounts.main]\nemail = \"{email}\"\n");
        let config = parse_with(MINIMAL, Some(&overlay)).unwrap();
        assert_eq!(config.base().email.as_deref(), Some(email.as_str()));
    }

    #[test]
    fn overlay_cannot_add_an_account() {
        let overlay = "[accounts.extra]\ndir = \"~/.claude-extra\"\n";
        let err = parse_with(MINIMAL, Some(overlay)).unwrap_err();
        assert_eq!(
            err,
            "config.local.toml: account `extra` is not in config.toml"
        );
    }

    // Pricing (#24).

    #[test]
    fn pricing_is_empty_by_default() {
        assert!(parse_ok(MINIMAL).pricing.is_empty());
    }

    #[test]
    fn pricing_entries_load_with_integers_and_fractions() {
        let config = parse_ok(&format!(
            "{MINIMAL}\n[pricing.claude-opus-5-5]\ninput = 4\noutput = 20\ncache_write = 5\ncache_read = 0.2\n"
        ));
        let price = config.pricing["claude-opus-5-5"];
        assert_eq!(
            price,
            Price {
                input: 4.0,
                output: 20.0,
                cache_write: 5.0,
                cache_read: 0.2,
            }
        );
    }

    #[test]
    fn pricing_needs_all_four_prices_and_no_other_key() {
        let missing = parse_err(&format!(
            "{MINIMAL}\n[pricing.m]\ninput = 1\noutput = 2\ncache_write = 3\n"
        ));
        assert!(missing.contains("cache_read"), "{missing}");
        let unknown = parse_err(&format!(
            "{MINIMAL}\n[pricing.m]\ninput = 1\noutput = 2\ncache_write = 3\ncache_read = 4\nbatch = 1\n"
        ));
        assert!(unknown.contains("batch"), "{unknown}");
    }

    #[test]
    fn pricing_rejects_a_negative_price() {
        let err = parse_err(&format!(
            "{MINIMAL}\n[pricing.m]\ninput = -1\noutput = 2\ncache_write = 3\ncache_read = 4\n"
        ));
        assert_eq!(
            err,
            "config.toml: pricing.m.input: must be a number that is 0 or more"
        );
    }

    #[test]
    fn overlay_can_change_one_price() {
        let main = format!(
            "{MINIMAL}\n[pricing.m]\ninput = 1\noutput = 2\ncache_write = 3\ncache_read = 4\n"
        );
        let config = parse_with(&main, Some("[pricing.m]\noutput = 9\n")).unwrap();
        assert_eq!(config.pricing["m"].output, 9.0);
        assert_eq!(config.pricing["m"].input, 1.0);
    }
}

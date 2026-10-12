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
const BASE_DIR: &str = "~/.claude";
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
    pub weight: Option<f64>,
    pub aliases: Vec<String>,
    pub enabled: bool,
    pub email: Option<String>,
    /// True for the one account whose dir is `~/.claude`.
    pub is_base: bool,
}

/// Thresholds in percent used, and stickiness in points.
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub session_avoid: u32,
    pub weekly_avoid: u32,
    pub scoped_avoid: u32,
    pub stickiness: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Usage {
    pub fresh_minutes: u64,
    pub fetch_timeout_secs: u64,
    pub refresh_min_gap_secs: u64,
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
    weight: Option<f64>,
    aliases: Option<Vec<String>>,
    enabled: Option<bool>,
    email: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPick {
    session_avoid: Option<u32>,
    weekly_avoid: Option<u32>,
    scoped_avoid: Option<u32>,
    stickiness: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawUsage {
    fresh_minutes: Option<u64>,
    fetch_timeout_secs: Option<u64>,
    refresh_min_gap_secs: Option<u64>,
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

fn config_dir(env: &Env) -> Result<PathBuf> {
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
    let main_text = std::fs::read_to_string(&main_path)
        .map_err(|e| format!("cannot read config {}: {e}", main_path.display()))?;
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
    let mut accounts = Vec::new();
    let mut dirs = BTreeSet::new();
    for (name, account) in raw.accounts {
        if !valid_name(&name) {
            return Err(format!(
                "accounts.{name}: the name must match ^[a-z0-9][a-z0-9_-]*$"
            ));
        }
        let dir = account
            .dir
            .ok_or_else(|| format!("accounts.{name}: missing key `dir`"))?;
        let dir = canonical(&dir, home);
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
        if let Some(weight) = account.weight
            && !(weight.is_finite() && weight > 0.0)
        {
            return Err(format!("accounts.{name}.weight: must be a positive number"));
        }
        accounts.push(Account {
            is_base: dir == base_dir,
            name,
            dir,
            weight: account.weight,
            aliases: account.aliases.unwrap_or_default(),
            enabled: account.enabled.unwrap_or(true),
            email: account.email,
        });
    }
    // Dirs are unique, so at most one account is the base account.
    if !accounts.iter().any(|a| a.is_base) {
        return Err(format!(
            "no account has dir `{BASE_DIR}` (the base account)"
        ));
    }

    let pick = raw.pick.unwrap_or_default();
    let pick = Pick {
        session_avoid: percent("pick.session_avoid", pick.session_avoid, 90)?,
        weekly_avoid: percent("pick.weekly_avoid", pick.weekly_avoid, 95)?,
        scoped_avoid: percent("pick.scoped_avoid", pick.scoped_avoid, 98)?,
        stickiness: percent("pick.stickiness", pick.stickiness, 10)?,
    };
    let usage = raw.usage.unwrap_or_default();
    let usage = Usage {
        fresh_minutes: usage.fresh_minutes.unwrap_or(15),
        fetch_timeout_secs: usage.fetch_timeout_secs.unwrap_or(5),
        refresh_min_gap_secs: usage.refresh_min_gap_secs.unwrap_or(60),
    };
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
    })
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
        assert_eq!(angel2.weight, None);
        assert!(angel2.aliases.is_empty());
        assert!(config.launch.banner);
        assert_eq!(config.launch.claude_bin, None);
        assert_eq!(config.base_repo, None);
    }

    #[test]
    fn empty_pick_and_usage_use_defaults() {
        let config = parse_ok(&format!("{MINIMAL}\n[pick]\n[usage]\n"));
        let pick = config.pick;
        let usage = config.usage;
        assert_eq!(
            [
                pick.session_avoid,
                pick.weekly_avoid,
                pick.scoped_avoid,
                pick.stickiness
            ],
            [90, 95, 98, 10]
        );
        assert_eq!(
            [
                usage.fresh_minutes,
                usage.fetch_timeout_secs,
                usage.refresh_min_gap_secs
            ],
            [15, 5, 60]
        );
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
            weight = 20

            [accounts.angel2]
            dir = "~/.claude-angel2"
            aliases = ["claude2"]
            enabled = false
            weight = 2.5

            [pick]
            session_avoid = 80
            weekly_avoid = 0
            scoped_avoid = 100
            stickiness = 5

            [usage]
            fresh_minutes = 1
            fetch_timeout_secs = 2
            refresh_min_gap_secs = 3
            "#,
        );
        assert_eq!(
            config.launch.claude_bin,
            Some(PathBuf::from("/home/test/bin/claude"))
        );
        assert!(!config.launch.banner);
        assert_eq!(
            config.base_repo.as_deref(),
            Some("https://github.com/example/claude-config.git")
        );
        assert_eq!(config.account("main").unwrap().weight, Some(20.0));
        let angel2 = config.account("angel2").unwrap();
        assert_eq!(angel2.aliases, ["claude2"]);
        assert!(!angel2.enabled);
        assert_eq!(angel2.weight, Some(2.5));
        assert_eq!(config.pick.session_avoid, 80);
        assert_eq!(config.pick.scoped_avoid, 100);
        assert_eq!(config.usage.refresh_min_gap_secs, 3);
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
    fn weight_must_be_positive() {
        for weight in ["0", "-1", "0.0"] {
            let err = parse_err(&format!(
                "[accounts.main]\ndir = \"~/.claude\"\nweight = {weight}\n"
            ));
            assert!(err.contains("weight"), "{weight}: {err}");
        }
    }

    #[test]
    fn pick_values_must_be_from_0_to_100() {
        for key in [
            "session_avoid",
            "weekly_avoid",
            "scoped_avoid",
            "stickiness",
        ] {
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
            "{MINIMAL}\n[accounts.x]\ndir = \"~/.x\"\nown = 1\n"
        ));
        assert!(err.contains("own"), "{err}");

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
}

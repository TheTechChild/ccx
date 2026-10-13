//! `ccx manage add NAME`: make a new account in one step.
//!
//! Add makes the account dir at mode 0700, links the shared setup of the
//! base account into it (#7), runs `claude auth login` in it, checks the
//! email and writes the account into the config. `config.toml` gets the
//! name and the dir. The email goes only into `config.local.toml`.
//!
//! ccx never reads or keeps Claude Code credentials. Only `claude auth
//! login` touches them.

use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use crate::adopt;
use crate::config::{self, Account, Config, Result, Source};
use crate::env::Env;
use crate::launch;
use crate::paths::{canonical, tilde};
use crate::pick;
use crate::prefs;
use crate::status::{self, Auth};
use crate::sync::{self, Class};

/// The mode of the new account dir.
const DIR_MODE: u32 = 0o700;
/// The mode of `config.local.toml`.
const OVERLAY_MODE: u32 = 0o600;
/// The mode of a new `config.toml`.
const CONFIG_MODE: u32 = 0o644;
/// The timeout of `claude auth status --json` after the login.
const STATUS_TIMEOUT: Duration = Duration::from_secs(30);

/// The arguments of `ccx manage add`.
#[derive(Debug, Clone, Default)]
pub struct Args {
    pub name: String,
    /// The account dir. The default is `~/.claude-NAME`.
    pub dir: Option<String>,
    /// The email to give to `claude auth login --email`.
    pub email: Option<String>,
    /// Stop after sync. Do not log in.
    pub no_login: bool,
    /// Do not copy the MCP servers and the preferences after the login.
    pub no_prefs: bool,
    /// Confirm the plan without a prompt.
    pub yes: bool,
    /// Make an API-key account, a paid fallback (#20).
    pub api_key: bool,
    /// With `api_key`: get the key from this command at each paid launch.
    pub key_command: Option<String>,
    /// With `api_key`: keep the key in the 0600 key file.
    pub key_file: bool,
}

/// The kind, the tier and the weight of an account after the login.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    pub kind: String,
    /// The display name of the tier, or the raw value for an unknown tier.
    pub tier: Option<String>,
    pub weight: f64,
}

impl Info {
    pub fn line(&self, name: &str) -> String {
        format!(
            "{name}: kind {}, tier {}, weight {}",
            self.kind,
            self.tier.as_deref().unwrap_or("-"),
            self.weight
        )
    }
}

/// Runs `ccx manage add`.
pub fn run(parent: &Env, args: &Args) -> Result<()> {
    if args.api_key {
        return crate::add_api_key::run(parent, args);
    }
    let home = config::home(parent)?;
    let config = config::load(parent)?;
    let config_path = config::config_path(parent)?;
    let overlay_path = config::overlay_path(parent)?;
    let cwd = std::env::current_dir().map_err(|e| format!("cannot read the cwd: {e}"))?;
    let (dir, dir_text) = account_dir(&args.name, args.dir.as_deref(), home, &cwd);
    check_new(&config, &args.name, &dir)?;
    check_dir(&dir)?;
    let account = Account {
        name: args.name.clone(),
        dir: dir.clone(),
        dir_text: dir_text.clone(),
        weight: None,
        aliases: Vec::new(),
        enabled: true,
        email: None,
        own: Vec::new(),
        is_base: false,
        api_key: None,
    };

    let depth = launch::check_depth(parent)?;
    let claude = if args.no_login {
        None
    } else {
        let self_exe = std::env::current_exe().ok();
        let claude = launch::find_claude(
            config.launch.claude_bin.as_deref(),
            parent,
            self_exe.as_deref(),
        );
        Some(claude.ok_or("cannot find claude; set launch.claude_bin or CCX_CLAUDE_BIN")?)
    };

    // The new config must load with the normal loader before add changes
    // a file.
    let mut main = adopt::read_doc(&config_path)?;
    let mut overlay = adopt::read_doc(&overlay_path)?;
    adopt::add_account(&mut main, &args.name, &dir_text)?;
    validate(&main, &overlay, &config_path, &overlay_path, home)?;

    println!("ccx manage add will:");
    for line in plan(args, &dir) {
        println!("  - {line}");
    }
    let retry = format!("ccx manage add {} -y", args.name);
    if !adopt::confirm(args.yes, "Continue?", &retry)? {
        println!("Nothing changed.");
        return Ok(());
    }

    make_dir(&dir)?;
    println!("Made {} (mode 0700)", dir.display());
    let sync_plan = sync::plan(&config.base().dir, &dir, &[])?;
    print!("{}", sync::render(&args.name, &sync_plan));
    let applied = sync::apply(&sync_plan, sync::Mode::Full { conflicts: false })?;
    println!("Sync made {} changes.", applied.changed);

    let mut email = args.email.clone();
    let mut info = None;
    if let Some(claude) = &claude {
        let (env, _) = launch::build_env(parent, &account, depth);
        login(claude, &env, args.email.as_deref())?;
        let text = status::auth_status_text(claude, &env, home, STATUS_TIMEOUT);
        let auth = text.as_deref().and_then(status::parse_auth);
        let got = text
            .as_deref()
            .and_then(|text| adopt::parse_email(text.as_bytes()));
        if let Some(warning) = email_warning(args.email.as_deref(), got.as_deref()) {
            let again = format!(
                "The config did not change. To log in again, run `ccx manage add {}` again. To keep this login, run `ccx manage adopt`",
                args.name
            );
            if !terminal() {
                return Err(format!("{warning}. {again}"));
            }
            eprintln!("ccx: warning: {warning}");
            if !adopt::confirm(false, "Write the account into the config?", "")? {
                println!("{again}.");
                return Ok(());
            }
        }
        // The email of the login is the true email of the account.
        email = got.or(email);
        info = Some(account_info(auth.as_ref(), &account, home));
    }

    if let Some(email) = &email {
        adopt::set_email(&mut overlay, &args.name, email)?;
    }
    let (main_text, overlay_text) = validate(&main, &overlay, &config_path, &overlay_path, home)?;
    let mode = fs::metadata(&config_path)
        .map(|m| m.permissions().mode() & 0o7777)
        .unwrap_or(CONFIG_MODE);
    let written = adopt::write_atomic(&config_path, &main_text, mode)?;
    println!("Wrote {}", written.display());
    if email.is_some() {
        let written = adopt::write_atomic(&overlay_path, &overlay_text, OVERLAY_MODE)?;
        println!("Wrote {} (mode 0600)", written.display());
    }

    match info {
        Some(info) => println!("{}", info.line(&args.name)),
        None => println!("No login. To log in, run `ccx @{} auth login`.", args.name),
    }
    if claude.is_some() && !args.no_prefs {
        copy_prefs(parent, &config, &account, claude);
    }
    Ok(())
}

/// Copies the user MCP servers and the allowlisted preferences of the base
/// account into the new account (#17). A failure is only a warning: the
/// account is in the config, and `ccx manage sync --prefs` can try again.
fn copy_prefs(parent: &Env, config: &Config, account: &Account, claude: Option<PathBuf>) {
    let copied = prefs::Copier::new(parent, config, claude)
        .and_then(|copier| copier.map_or(Ok(true), |c| c.copy(account, false)));
    let error = match copied {
        Ok(true) => return,
        Ok(false) => None,
        Err(e) => Some(e),
    };
    if let Some(e) = error {
        eprintln!("ccx: warning: {e}");
    }
    eprintln!(
        "ccx: warning: the copy of the MCP servers and the preferences is not complete. To try again, run `ccx manage sync --prefs {}`.",
        account.name
    );
}

/// The canonical dir and its text for `config.toml`. A dir in HOME starts
/// with `~`. A relative `--dir` is relative to `cwd`.
pub fn account_dir(name: &str, dir: Option<&str>, home: &OsStr, cwd: &Path) -> (PathBuf, String) {
    let text = dir.map_or_else(|| format!("~/.claude-{name}"), str::to_string);
    let mut path = canonical(&text, home);
    if path.is_relative() {
        path = canonical(&cwd.join(&path).to_string_lossy(), home);
    }
    let text = tilde(&path, home);
    (path, text)
}

/// Refuses a name that is not valid or in use, and a dir that an account
/// uses.
pub fn check_new(config: &Config, name: &str, dir: &Path) -> Result<()> {
    if !config::valid_name(name) {
        return Err(format!(
            "`{name}` is not a valid account name; the name must match ^[a-z0-9][a-z0-9_-]*$"
        ));
    }
    if config.account(name).is_some() {
        return Err(format!("the account `{name}` already exists"));
    }
    if let Some(other) = config.accounts.iter().find(|a| a.dir == dir) {
        return Err(format!(
            "the account `{}` already uses {}",
            other.name,
            dir.display()
        ));
    }
    Ok(())
}

/// Refuses a dir that exists and has other files than Claude Code files.
/// A dir with only Claude Code files is from an earlier `manage add` that
/// stopped, for example when the login failed.
pub fn check_dir(dir: &Path) -> Result<()> {
    let meta = match fs::symlink_metadata(dir) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("cannot read {}: {e}", dir.display())),
    };
    if !meta.is_dir() {
        return Err(format!("{} exists and is not a directory", dir.display()));
    }
    for name in sync::names(dir)? {
        let text = name.to_string_lossy();
        if sync::classify(&text) == Class::Unknown {
            return Err(format!(
                "{} exists and has files that are not Claude Code files, for example `{text}`; to use an account dir that exists, run `ccx manage adopt`",
                dir.display()
            ));
        }
    }
    Ok(())
}

/// The plan lines that add shows before it asks.
pub fn plan(args: &Args, dir: &Path) -> Vec<String> {
    let mut lines = vec![
        format!("make {} with mode 0700", dir.display()),
        "link the shared setup of the base account into it (sync)".to_string(),
    ];
    if !args.no_login {
        let email = args
            .email
            .as_deref()
            .map(|e| format!(" --email {e}"))
            .unwrap_or_default();
        lines.push(format!(
            "run `claude auth login{email}` with CLAUDE_CONFIG_DIR={}",
            dir.display()
        ));
        lines.push("compare the email of the login with `claude auth status`".to_string());
    }
    lines.push(format!("add account {} to config.toml", args.name));
    if !args.no_login || args.email.is_some() {
        lines.push("write the email only to config.local.toml (mode 0600)".to_string());
    }
    if !args.no_login && !args.no_prefs {
        lines.push(
            "copy the user MCP servers (`claude mcp add-json -s user`) and the allowlisted preferences of the base account"
                .to_string(),
        );
    }
    lines
}

/// Makes `dir` and its missing parents. The dir gets mode 0700, also when
/// it exists.
pub fn make_dir(dir: &Path) -> Result<()> {
    let error = |e: std::io::Error| format!("cannot make {}: {e}", dir.display());
    fs::DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(dir)
        .map_err(error)?;
    // The umask can remove bits, and a dir that exists keeps its mode.
    fs::set_permissions(dir, fs::Permissions::from_mode(DIR_MODE)).map_err(error)
}

fn terminal() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// Runs `claude auth login` on the terminal of the user.
fn login(claude: &Path, env: &Env, email: Option<&str>) -> Result<()> {
    crate::lifecycle::note_no_browser(env);
    let mut command = Command::new(claude);
    command.args(["auth", "login"]);
    if let Some(email) = email {
        command.args(["--email", email]);
    }
    let status = command
        .env_clear()
        .envs(env.iter())
        .status()
        .map_err(|e| format!("cannot run {}: {e}", claude.display()))?;
    if !status.success() {
        return Err(format!(
            "`claude auth login` failed ({status}); the config did not change. To try again, run `ccx manage add` again"
        ));
    }
    Ok(())
}

/// A warning when the login has no email or a different email than
/// `--email`. Emails compare with no difference between cases.
pub fn email_warning(want: Option<&str>, got: Option<&str>) -> Option<String> {
    match (want, got) {
        (_, None) => Some("`claude auth status` shows no login with an email".to_string()),
        (Some(want), Some(got)) if !want.eq_ignore_ascii_case(got) => {
            Some(format!("the login is {got}, but --email is {want}"))
        }
        _ => None,
    }
}

/// The kind from `claude auth status`, and the tier and the weight from the
/// `.claude.json` of the account.
pub fn account_info(auth: Option<&Auth>, account: &Account, home: &OsStr) -> Info {
    let claude_json: Option<Value> = fs::read(crate::usage::claude_json_path(account, home))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    info(auth, claude_json.as_ref(), account.weight)
}

/// `account_info` with the data read. `weight` is the weight in the config.
pub fn info(auth: Option<&Auth>, claude_json: Option<&Value>, weight: Option<f64>) -> Info {
    let text_at = |pointer: &str| {
        claude_json
            .and_then(|v| v.pointer(pointer))
            .and_then(Value::as_str)
    };
    let tier = text_at("/oauthAccount/organizationRateLimitTier");
    Info {
        kind: auth.map_or_else(|| "-".into(), status::kind_label),
        tier: tier.map(status::tier_label),
        weight: weight
            .unwrap_or_else(|| pick::tier_weight(text_at("/oauthAccount/organizationType"), tier)),
    }
}

/// Checks the edited files with the normal loader. Returns their text.
pub fn validate(
    main: &toml_edit::DocumentMut,
    overlay: &toml_edit::DocumentMut,
    main_path: &Path,
    overlay_path: &Path,
    home: &OsStr,
) -> Result<(String, String)> {
    let (main_text, overlay_text) = (main.to_string(), overlay.to_string());
    config::parse(
        &Source {
            path: main_path,
            text: &main_text,
        },
        Some(&Source {
            path: overlay_path,
            text: &overlay_text,
        }),
        home,
    )?;
    Ok((main_text, overlay_text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HOME: &str = "/home/test";

    fn config() -> Config {
        let text = "[accounts.main]\ndir = \"~/.claude\"\n\n[accounts.angel2]\ndir = \"~/.claude-angel2\"\n";
        config::parse(
            &Source {
                path: Path::new("config.toml"),
                text,
            },
            None,
            OsStr::new(HOME),
        )
        .unwrap()
    }

    fn dir_of(name: &str, dir: Option<&str>) -> (PathBuf, String) {
        account_dir(name, dir, OsStr::new(HOME), Path::new("/work"))
    }

    #[test]
    fn account_dir_defaults_to_claude_dash_name() {
        assert_eq!(
            dir_of("work", None),
            (
                PathBuf::from("/home/test/.claude-work"),
                "~/.claude-work".into()
            )
        );
        assert_eq!(
            dir_of("work", Some("~/accounts/w/")),
            (
                PathBuf::from("/home/test/accounts/w"),
                "~/accounts/w".into()
            )
        );
        assert_eq!(
            dir_of("work", Some("/opt/x")),
            (PathBuf::from("/opt/x"), "/opt/x".into())
        );
        assert_eq!(
            dir_of("work", Some("rel/../dir")),
            (PathBuf::from("/work/dir"), "/work/dir".into())
        );
    }

    #[test]
    fn check_new_refuses_bad_names_and_duplicates() {
        let config = config();
        let free = Path::new("/home/test/.claude-new");
        assert!(check_new(&config, "new", free).is_ok());
        assert!(check_new(&config, "a_b-9", free).is_ok());
        for bad in ["", "Bad", "-x", "_x", "a.b", "a b", "a/b"] {
            let e = check_new(&config, bad, free).unwrap_err();
            assert!(e.contains("not a valid account name"), "{bad}: {e}");
        }
        let e = check_new(&config, "angel2", free).unwrap_err();
        assert!(e.contains("already exists"), "{e}");
        let e = check_new(&config, "new", Path::new("/home/test/.claude-angel2")).unwrap_err();
        assert!(e.contains("`angel2` already uses"), "{e}");
        let e = check_new(&config, "new", Path::new("/home/test/.claude")).unwrap_err();
        assert!(e.contains("`main` already uses"), "{e}");
    }

    #[test]
    fn check_dir_accepts_a_missing_dir_or_claude_files_only() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("acct");
        assert!(check_dir(&dir).is_ok());
        fs::create_dir(&dir).unwrap();
        assert!(check_dir(&dir).is_ok());
        std::os::unix::fs::symlink("/x/settings.json", dir.join("settings.json")).unwrap();
        fs::create_dir(dir.join("skills")).unwrap();
        fs::write(dir.join(".claude.json"), "{}").unwrap();
        assert!(check_dir(&dir).is_ok());
        fs::write(dir.join("notes.txt"), "").unwrap();
        let e = check_dir(&dir).unwrap_err();
        assert!(e.contains("`notes.txt`"), "{e}");
        let file = root.path().join("file");
        fs::write(&file, "").unwrap();
        assert!(check_dir(&file).unwrap_err().contains("not a directory"));
    }

    #[test]
    fn plan_names_each_step() {
        let mut args = Args {
            name: "work".into(),
            email: Some("work@example.com".into()),
            ..Args::default()
        };
        let dir = Path::new("/home/test/.claude-work");
        let text = plan(&args, dir).join("\n");
        assert!(text.contains("make /home/test/.claude-work with mode 0700"));
        assert!(text.contains("(sync)"));
        assert!(text.contains(
            "run `claude auth login --email work@example.com` with CLAUDE_CONFIG_DIR=/home/test/.claude-work"
        ));
        assert!(text.contains("config.local.toml"));
        assert!(text.contains("claude mcp add-json -s user"));

        args.no_prefs = true;
        let text = plan(&args, dir).join("\n");
        assert!(!text.contains("mcp add-json"), "{text}");

        args.no_login = true;
        args.email = None;
        let text = plan(&args, dir).join("\n");
        assert!(!text.contains("auth login"), "{text}");
        assert!(!text.contains("config.local.toml"), "{text}");
        assert!(text.contains("add account work to config.toml"));
    }

    #[test]
    fn email_warning_on_a_mismatch_or_no_login() {
        assert_eq!(email_warning(None, Some("a@example.com")), None);
        assert_eq!(
            email_warning(Some("A@Example.com"), Some("a@example.com")),
            None
        );
        let w = email_warning(Some("a@example.com"), Some("b@example.com")).unwrap();
        assert!(
            w.contains("b@example.com") && w.contains("a@example.com"),
            "{w}"
        );
        assert!(email_warning(Some("a@example.com"), None).is_some());
        assert!(email_warning(None, None).is_some());
    }

    #[test]
    fn info_shows_kind_tier_and_weight() {
        let auth = Auth {
            logged_in: true,
            method: Some("claude.ai".into()),
            subscription: Some("max".into()),
        };
        let max20 =
            json!({"oauthAccount": {"organizationRateLimitTier": "default_claude_max_20x"}});
        assert_eq!(
            info(Some(&auth), Some(&max20), None),
            Info {
                kind: "max".into(),
                tier: Some("Max 20x".into()),
                weight: 20.0
            }
        );
        // An unknown tier shows the raw string and the default weight.
        let odd = json!({"oauthAccount": {"organizationRateLimitTier": "new_tier_x"}});
        let got = info(Some(&auth), Some(&odd), None);
        assert_eq!(got.tier.as_deref(), Some("new_tier_x"));
        assert_eq!(got.weight, 5.0);
        // The config weight wins.
        assert_eq!(info(Some(&auth), Some(&max20), Some(2.5)).weight, 2.5);
        let pro = json!({"oauthAccount": {"organizationType": "claude_pro"}});
        let got = info(None, Some(&pro), None);
        assert_eq!(
            (got.kind.as_str(), got.tier.as_deref(), got.weight),
            ("-", None, 1.0)
        );
        assert_eq!(got.line("work"), "work: kind -, tier -, weight 1");
    }
}

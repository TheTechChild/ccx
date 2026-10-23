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
use crate::prefs;
use crate::status::{self, Auth};
use crate::sync::{self, Class};
use crate::wizard::{Ask, Prompt, Step, Stop};

/// The mode of the new account dir.
const DIR_MODE: u32 = 0o700;
/// The mode of `config.local.toml`.
const OVERLAY_MODE: u32 = 0o600;
/// The mode of a new `config.toml`.
const CONFIG_MODE: u32 = 0o644;
/// The timeout of `claude auth status --json` after the login.
const STATUS_TIMEOUT: Duration = Duration::from_secs(30);

/// The arguments of `ccx manage add`.
#[derive(Debug, Clone, Default, PartialEq)]
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

/// The kind and the tier of an account after the login.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    pub kind: String,
    /// The display name of the tier, or the raw value for an unknown tier.
    pub tier: Option<String>,
}

impl Info {
    pub fn line(&self, name: &str) -> String {
        format!(
            "{name}: kind {}, tier {}",
            self.kind,
            self.tier.as_deref().unwrap_or("-"),
        )
    }
}

/// The usage line that names the flags of `ccx manage add`.
pub const USAGE: &str =
    "ccx manage add NAME [--dir DIR] [--email EMAIL] [--no-login] [--no-prefs] [--api-key] -y";

/// Runs `ccx manage add`. `ask` gives the confirm before the first change.
pub fn run(parent: &Env, args: &Args, ask: &mut Ask) -> Result<()> {
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
    if !ask.confirm("Continue?", &retry)? {
        println!("Nothing changed.");
        return Ok(());
    }

    make_dir(&dir)?;
    println!("Made {} (mode 0700)", dir.display());
    let sync_plan = sync::plan(&config.base().dir, &dir, &[], &config.sync)?;
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
            if !ask.is_wizard() && !terminal() {
                return Err(format!("{warning}. {again}"));
            }
            eprintln!("ccx: warning: {warning}");
            let question = "Write the account into the config?";
            let write = match ask {
                Ask::Wizard(_) => ask.confirm(question, "")?,
                // `-y` confirms the plan, not a different login.
                Ask::Flags { .. } => adopt::confirm(false, question, "")?,
            };
            if !write {
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

    println!(
        "Summary: account {}, dir {dir_text}, email {}",
        args.name,
        email.as_deref().unwrap_or("-")
    );
    match info {
        Some(info) => println!("{}", info.line(&args.name)),
        None => println!("No login. To log in, run `ccx @{} auth login`.", args.name),
    }
    if claude.is_some() && !args.no_prefs {
        copy_prefs(parent, &config, &account, claude);
    }
    Ok(())
}

/// `ccx manage add` with no name on a terminal: the add wizard. The
/// values in `given` (from the flags) are the defaults of the steps.
pub fn run_wizard(parent: &Env, given: &Args, p: &mut dyn Prompt) -> Step<()> {
    let home = config::home(parent)?;
    let config = config::load(parent)?;
    let cwd = std::env::current_dir().map_err(|e| format!("cannot read the cwd: {e}"))?;
    let args = wizard(p, &config, home, &cwd, given)?;
    run(parent, &args, &mut Ask::Wizard(p)).map_err(Stop::Fail)
}

/// The steps of the add wizard: name, config dir, sync, login. Returns the
/// same [`Args`] that the flags give for these values. The summary and the
/// confirm come after, in [`run`], before the first change.
pub fn wizard(
    p: &mut dyn Prompt,
    config: &Config,
    home: &OsStr,
    cwd: &Path,
    given: &Args,
) -> Step<Args> {
    p.show("ccx manage add: make a new account. Push Esc or Ctrl-C to stop. Nothing changes before the summary.");
    let accounts: Vec<String> = config
        .accounts
        .iter()
        .map(|a| format!("{} ({})", a.name, a.dir_text))
        .collect();
    p.show(&format!("Accounts now: {}", accounts.join(", ")));

    let name = p.text("Step 1 of 4: the account name", None, &|name| {
        let (dir, _) = account_dir(name, None, home, cwd);
        check_new(config, name, &dir).or_else(|e| {
            // A used default dir is not a problem yet: step 2 can change it.
            if e.contains("already uses") {
                Ok(())
            } else {
                Err(e)
            }
        })
    })?;
    let default_dir = format!("~/.claude-{name}");
    let dir = p.text(
        "Step 2 of 4: the config dir",
        Some(given.dir.as_deref().unwrap_or(&default_dir)),
        &|text| {
            let (dir, _) = account_dir(&name, Some(text), home, cwd);
            check_new(config, &name, &dir)?;
            check_dir(&dir)
        },
    )?;
    p.show(&format!(
        "Step 3 of 4: sync. ccx links the shared setup of the base account into the new dir: {}",
        shared_items(&config.base().dir, &config.sync)
    ));
    let mut args = Args {
        name,
        dir: (dir != default_dir).then_some(dir),
        yes: false,
        ..given.clone()
    };
    if args.api_key {
        p.show("Step 4 of 4: API key. ccx asks for the key after the summary.");
        return Ok(args);
    }
    let login = p.confirm(
        "Step 4 of 4: log in now with `claude auth login`?",
        !given.no_login,
    )?;
    args.no_login = !login;
    let email = p.text(
        "The expected email of the login (empty for none)",
        given.email.as_deref(),
        &|email| {
            if email.is_empty() || crate::lifecycle::valid_email(email) {
                Ok(())
            } else {
                Err(format!("`{email}` is not a valid email address"))
            }
        },
    )?;
    args.email = (!email.is_empty()).then_some(email);
    if login {
        let copy = p.confirm(
            "Copy the user MCP servers and the preferences of the base account?",
            !given.no_prefs,
        )?;
        args.no_prefs = !copy;
    }
    Ok(args)
}

/// The names of the base entries that sync links, or a note when there
/// are none.
fn shared_items(base: &Path, lists: &config::SyncLists) -> String {
    let names: Vec<String> = sync::names(base)
        .unwrap_or_default()
        .iter()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| {
            matches!(
                sync::classify_with(name, lists),
                Class::Shared | Class::Skills
            )
        })
        .collect();
    if names.is_empty() {
        "no shared items now".to_string()
    } else {
        names.join(", ")
    }
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

/// The kind from `claude auth status`, and the tier from the `.claude.json`
/// of the account.
pub fn account_info(auth: Option<&Auth>, account: &Account, home: &OsStr) -> Info {
    let claude_json: Option<Value> = fs::read(crate::usage::claude_json_path(account, home))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    info(auth, claude_json.as_ref())
}

/// `account_info` with the data read.
pub fn info(auth: Option<&Auth>, claude_json: Option<&Value>) -> Info {
    let tier = claude_json
        .and_then(|v| v.pointer("/oauthAccount/organizationRateLimitTier"))
        .and_then(Value::as_str);
    Info {
        kind: auth.map_or_else(|| "-".into(), status::kind_label),
        tier: tier.map(status::tier_label),
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
    fn info_shows_kind_and_tier() {
        let auth = Auth {
            logged_in: true,
            method: Some("claude.ai".into()),
            subscription: Some("max".into()),
        };
        let max20 =
            json!({"oauthAccount": {"organizationRateLimitTier": "default_claude_max_20x"}});
        assert_eq!(
            info(Some(&auth), Some(&max20)),
            Info {
                kind: "max".into(),
                tier: Some("Max 20x".into()),
            }
        );
        // An unknown tier shows the raw string.
        let odd = json!({"oauthAccount": {"organizationRateLimitTier": "new_tier_x"}});
        let got = info(Some(&auth), Some(&odd));
        assert_eq!(got.tier.as_deref(), Some("new_tier_x"));
        let pro = json!({"oauthAccount": {"organizationType": "claude_pro"}});
        let got = info(None, Some(&pro));
        assert_eq!((got.kind.as_str(), got.tier.as_deref()), ("-", None));
        assert_eq!(got.line("work"), "work: kind -, tier -");
    }

    use crate::wizard::Stop;
    use crate::wizard::fake::{Reply, Script};

    fn ask(replies: Vec<Reply>, given: &Args) -> (Step<Args>, Script) {
        let mut p = Script::new(replies);
        let got = wizard(
            &mut p,
            &config(),
            OsStr::new(HOME),
            Path::new("/work"),
            given,
        );
        (got, p)
    }

    #[test]
    fn wizard_gives_the_same_args_and_plan_as_the_flags() {
        let (got, p) = ask(
            vec![
                Reply::Text("Bad"),
                Reply::Text("angel2"),
                Reply::Text("work"),
                Reply::Text(""),
                Reply::Yes,
                Reply::Text("not-an-email"),
                Reply::Text("work@example.com"),
                Reply::No,
            ],
            &Args::default(),
        );
        assert!(p.done());
        let log = p.text_log();
        assert!(
            log.contains("Accounts now: angel2 (~/.claude-angel2), main (~/.claude)"),
            "{log}"
        );
        assert!(
            log.contains("error: `Bad` is not a valid account name"),
            "{log}"
        );
        assert!(
            log.contains("error: the account `angel2` already exists"),
            "{log}"
        );
        assert!(
            log.contains("error: `not-an-email` is not a valid email"),
            "{log}"
        );
        assert!(log.contains("Step 3 of 4: sync"), "{log}");
        // `ccx manage add work --email work@example.com --no-prefs`
        let flags = Args {
            name: "work".into(),
            email: Some("work@example.com".into()),
            no_prefs: true,
            ..Args::default()
        };
        let got = got.unwrap();
        assert_eq!(got, flags);
        let dir = Path::new("/home/test/.claude-work");
        assert_eq!(plan(&got, dir), plan(&flags, dir));
    }

    #[test]
    fn wizard_with_a_dir_and_no_login() {
        let (got, p) = ask(
            vec![
                Reply::Text("w2"),
                Reply::Text("~/.claude-angel2"),
                Reply::Text("~/accts/w2"),
                Reply::No,
                Reply::Text(""),
            ],
            &Args::default(),
        );
        assert!(p.done());
        assert!(
            p.text_log()
                .contains("error: the account `angel2` already uses")
        );
        // `ccx manage add w2 --dir ~/accts/w2 --no-login`
        let flags = Args {
            name: "w2".into(),
            dir: Some("~/accts/w2".into()),
            no_login: true,
            ..Args::default()
        };
        assert_eq!(got.unwrap(), flags);
    }

    #[test]
    fn wizard_uses_the_given_flags_as_defaults() {
        let given = Args {
            dir: Some("/opt/acct".into()),
            email: Some("x@example.com".into()),
            yes: true,
            ..Args::default()
        };
        let (got, _) = ask(
            vec![
                Reply::Text("x"),
                Reply::Text(""),
                Reply::Yes,
                Reply::Text(""),
                Reply::Yes,
            ],
            &given,
        );
        let got = got.unwrap();
        assert_eq!(got.dir.as_deref(), Some("/opt/acct"));
        assert_eq!(got.email.as_deref(), Some("x@example.com"));
        // The wizard always shows the summary and asks.
        assert!(!got.yes);
        // An API-key account has no login steps.
        let given = Args {
            api_key: true,
            ..Args::default()
        };
        let (got, p) = ask(vec![Reply::Text("paid"), Reply::Text("")], &given);
        assert!(p.done());
        assert!(got.unwrap().api_key);
    }

    #[test]
    fn esc_at_a_step_stops_the_wizard() {
        let (got, _) = ask(vec![Reply::Text("work"), Reply::Esc], &Args::default());
        assert_eq!(got, Err(Stop::Cancel));
        let (got, _) = ask(
            vec![Reply::Text("work"), Reply::Text(""), Reply::Esc],
            &Args::default(),
        );
        assert_eq!(got, Err(Stop::Cancel));
    }

    /// A temp HOME with a base account, the config and the fake claude.
    struct Home {
        dir: tempfile::TempDir,
    }

    impl Home {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let base = dir.path().join(".claude");
            fs::create_dir_all(base.join("skills/tdd")).unwrap();
            fs::write(base.join("settings.json"), "{}\n").unwrap();
            let config = dir.path().join(".config/ccx");
            fs::create_dir_all(&config).unwrap();
            fs::write(
                config.join("config.toml"),
                "[accounts.main]\ndir = \"~/.claude\"\n",
            )
            .unwrap();
            Home { dir }
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }

        fn env(&self) -> Env {
            let fakebin = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fakebin");
            [
                ("HOME", self.path().as_os_str().to_owned()),
                (
                    "PATH",
                    format!("{}:/usr/bin:/bin", fakebin.display()).into(),
                ),
                (
                    "FAKE_CLAUDE_LOG",
                    self.path().join("fake-claude.log").into_os_string(),
                ),
            ]
            .into_iter()
            .collect()
        }

        fn read(&self, path: &str) -> String {
            fs::read_to_string(self.path().join(path)).unwrap_or_default()
        }
    }

    /// End to end: the add wizard with the fake claude.
    #[test]
    fn add_wizard_end_to_end_with_the_fake_claude() {
        let home = Home::new();
        let mut p = Script::new([
            Reply::Text("work"),
            Reply::Text(""),
            Reply::Yes,
            Reply::Text("work@example.com"),
            Reply::No,
            // The summary: write the changes.
            Reply::Yes,
        ]);
        run_wizard(&home.env(), &Args::default(), &mut p).unwrap();
        assert!(p.done());
        assert!(
            p.text_log().contains("settings.json, skills"),
            "{}",
            p.text_log()
        );
        let dir = home.path().join(".claude-work");
        assert!(dir.join("settings.json").is_symlink());
        assert_eq!(
            fs::read_to_string(dir.join(".fake-login")).unwrap(),
            "work@example.com"
        );
        let config = home.read(".config/ccx/config.toml");
        assert!(config.contains("[accounts.work]"), "{config}");
        assert!(!config.contains("@"), "{config}");
        let overlay = home.read(".config/ccx/config.local.toml");
        assert!(
            overlay.contains("email = \"work@example.com\""),
            "{overlay}"
        );
        let log = home.read("fake-claude.log");
        assert!(log.contains("\"login\""), "{log}");
        assert!(!log.contains("\"mcp\""), "{log}");
    }

    /// Esc at the summary or at a step changes no file.
    #[test]
    fn add_wizard_cancel_changes_nothing() {
        let home = Home::new();
        let before = home.read(".config/ccx/config.toml");
        for replies in [
            vec![Reply::Text("work"), Reply::Esc],
            vec![
                Reply::Text("work"),
                Reply::Text(""),
                Reply::No,
                Reply::Text(""),
                Reply::Esc,
            ],
            vec![
                Reply::Text("work"),
                Reply::Text(""),
                Reply::No,
                Reply::Text(""),
                Reply::No,
            ],
        ] {
            let mut p = Script::new(replies);
            let got = run_wizard(&home.env(), &Args::default(), &mut p);
            assert!(matches!(got, Ok(()) | Err(Stop::Cancel)), "{got:?}");
            assert!(p.done());
            assert_eq!(home.read(".config/ccx/config.toml"), before);
            assert!(!home.path().join(".config/ccx/config.local.toml").exists());
            assert!(!home.path().join(".claude-work").exists());
            assert_eq!(home.read("fake-claude.log"), "");
        }
    }
}

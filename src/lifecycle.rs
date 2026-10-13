//! `ccx manage login`, `logout`, `edit`, `edit-config` and `rm`: the
//! account lifecycle after `ccx manage add`.
//!
//! All logins and logouts go through `claude auth login` and `claude auth
//! logout` with the launch environment of the account (ADR 0002). ccx never
//! reads, keeps or refreshes Claude Code credentials. `rm --purge` only
//! deletes the credentials file. It never reads it.
//!
//! `config.toml` never gets an email. The expected email of an account
//! comes only from `config.local.toml` (decision D2).

use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use toml_edit::{DocumentMut, Item, Value};

use crate::add;
use crate::adopt;
use crate::config::{self, Account, Config, Result};
use crate::doctor;
use crate::env::Env;
use crate::launch;
use crate::status;
use crate::usage;

/// The timeout of `claude auth status --json`.
const STATUS_TIMEOUT: Duration = Duration::from_secs(30);
/// The mode of `config.local.toml`.
const OVERLAY_MODE: u32 = 0o600;
/// The mode of a new `config.toml`.
const CONFIG_MODE: u32 = 0o644;
/// The mode of the trash dir.
const TRASH_MODE: u32 = 0o700;
/// The file that Claude Code keeps the login in when there is no keychain.
/// `rm --purge` only deletes it. The guard test in
/// `tests/no_credentials.rs` allows this name only with `remove_file`.
const CREDENTIALS_FILE: &str = ".credentials.json";

/// The paths and the loaded config that each command needs.
struct Setup<'a> {
    parent: &'a Env,
    home: &'a OsStr,
    config: Config,
    config_path: PathBuf,
    overlay_path: PathBuf,
}

impl<'a> Setup<'a> {
    fn load(parent: &'a Env) -> Result<Self> {
        Ok(Setup {
            parent,
            home: config::home(parent)?,
            config: config::load(parent)?,
            config_path: config::config_path(parent)?,
            overlay_path: config::overlay_path(parent)?,
        })
    }

    fn account(&self, name: &str) -> Result<&Account> {
        self.config.account(name).ok_or_else(|| {
            format!(
                "no account `{name}` in the config; the accounts are: {}",
                self.config.names().join(", ")
            )
        })
    }

    fn claude(&self) -> Result<PathBuf> {
        let self_exe = std::env::current_exe().ok();
        launch::find_claude(
            self.config.launch.claude_bin.as_deref(),
            self.parent,
            self_exe.as_deref(),
        )
        .ok_or_else(|| "cannot find claude; set launch.claude_bin or CCX_CLAUDE_BIN".to_string())
    }

    /// The launch environment of `account`: `CLAUDE_CONFIG_DIR` is the
    /// account dir, or not set for the base account.
    fn env(&self, account: &Account) -> Result<Env> {
        let depth = launch::check_depth(self.parent)?;
        Ok(launch::build_env(self.parent, account, depth).0)
    }
}

/// `ccx manage login NAME` or `ccx manage login --all`. Returns false when
/// a login of `--all` failed.
pub fn login(parent: &Env, name: Option<&str>) -> Result<bool> {
    let Some(name) = name else {
        return Ok(login_missing(parent)?.ok);
    };
    let setup = Setup::load(parent)?;
    let claude = setup.claude()?;
    let overlay = adopt::read_doc(&setup.overlay_path)?;
    let account = setup.account(name)?;
    let line = login_one(&setup, &claude, account, adopt::email_of(&overlay, name))?;
    println!("{line}");
    Ok(true)
}

/// The result of [`login_missing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Logins {
    /// False when a login failed.
    pub ok: bool,
    /// The number of logins that ccx did.
    pub done: usize,
}

/// Logs in to each account that `claude auth status` shows as not logged
/// in, with the email check. Then shows one summary line for each account.
pub fn login_missing(parent: &Env) -> Result<Logins> {
    let setup = Setup::load(parent)?;
    let claude = setup.claude()?;
    let overlay = adopt::read_doc(&setup.overlay_path)?;
    let mut summary = Vec::new();
    let mut logins = Logins { ok: true, done: 0 };
    for account in &setup.config.accounts {
        let env = setup.env(account)?;
        let text = status::auth_status_text(&claude, &env, setup.home, STATUS_TIMEOUT);
        if text
            .as_deref()
            .and_then(status::parse_auth)
            .is_some_and(|auth| auth.logged_in)
        {
            summary.push(format!("{}: logged in already", account.name));
            continue;
        }
        let expected = adopt::email_of(&overlay, &account.name);
        logins.done += 1;
        match login_one(&setup, &claude, account, expected) {
            Ok(line) => summary.push(line),
            Err(e) => {
                logins.ok = false;
                summary.push(format!("{}: FAILED: {e}", account.name));
            }
        }
    }
    println!("Summary:");
    for line in summary {
        println!("  {line}");
    }
    Ok(logins)
}

/// True when claude cannot open a browser for the login: in an SSH
/// session, or on Linux with no `DISPLAY` and no `WAYLAND_DISPLAY`.
pub fn no_browser(env: &Env, linux: bool) -> bool {
    env.get("SSH_CONNECTION").is_some()
        || env.get("SSH_TTY").is_some()
        || (linux && env.get("DISPLAY").is_none() && env.get("WAYLAND_DISPLAY").is_none())
}

/// The note that ccx shows before a login when there is no browser.
pub const NO_BROWSER_NOTE: &str = "\
ccx: no browser is available. claude prints a login URL.
ccx: open the URL on a different device and log in.
ccx: then copy the code and paste it into the claude prompt here.";

/// Shows [`NO_BROWSER_NOTE`] when there is no browser.
pub fn note_no_browser(env: &Env) {
    if no_browser(env, cfg!(target_os = "linux")) {
        println!("{NO_BROWSER_NOTE}");
    }
}

/// Runs `claude auth login [--email E]` for `account` and compares the
/// email of the login with `expected`. Returns the summary line.
fn login_one(
    setup: &Setup,
    claude: &Path,
    account: &Account,
    expected: Option<&str>,
) -> Result<String> {
    let name = &account.name;
    let env = setup.env(account)?;
    if expected.is_none() {
        eprintln!(
            "ccx: warning: config.local.toml has no email for account {name}; ccx does not check the email of the login"
        );
    }
    println!("Log in to account {name} ({})", account.dir.display());
    note_no_browser(&env);
    let mut args = vec!["auth", "login"];
    if let Some(email) = expected {
        args.extend(["--email", email]);
    }
    run_claude(claude, &env, &args)?;
    let text = status::auth_status_text(claude, &env, setup.home, STATUS_TIMEOUT);
    let got = text
        .as_deref()
        .and_then(|text| adopt::parse_email(text.as_bytes()));
    let Some(expected) = expected else {
        return Ok(format!("{name}: logged in (email not checked)"));
    };
    check_email(name, expected, got.as_deref())?;
    Ok(format!("{name}: logged in as {expected}"))
}

/// An error when the login has no email or a different email than the
/// expected email. Emails compare with no difference between cases.
pub fn check_email(name: &str, expected: &str, got: Option<&str>) -> Result<()> {
    let logout = format!("To log out, run `ccx manage logout {name}`");
    match got {
        Some(got) if got.eq_ignore_ascii_case(expected) => Ok(()),
        Some(got) => Err(format!(
            "the login of account {name} is {got}, but config.local.toml expects {expected}. {logout}"
        )),
        None => Err(format!(
            "`claude auth status` shows no login with an email for account {name}"
        )),
    }
}

/// `ccx manage logout NAME`: runs `claude auth logout` for that account.
pub fn logout(parent: &Env, name: &str, yes: bool) -> Result<()> {
    let setup = Setup::load(parent)?;
    let account = setup.account(name)?;
    let claude = setup.claude()?;
    let retry = format!("ccx manage logout {name} -y");
    if !adopt::confirm(yes, &format!("Log out of account {name}?"), &retry)? {
        println!("Nothing changed.");
        return Ok(());
    }
    run_claude(&claude, &setup.env(account)?, &["auth", "logout"])?;
    println!("Logged out of account {name}.");
    Ok(())
}

/// Runs `claude ARGS` on the terminal of the user.
fn run_claude(claude: &Path, env: &Env, args: &[&str]) -> Result<()> {
    let status = Command::new(claude)
        .args(args)
        .env_clear()
        .envs(env.iter())
        .status()
        .map_err(|e| format!("cannot run {}: {e}", claude.display()))?;
    if !status.success() {
        return Err(format!("`claude {}` failed ({status})", args.join(" ")));
    }
    Ok(())
}

/// The changes of `ccx manage edit`.
#[derive(Debug, Clone, Default)]
pub struct Edit {
    pub email: Option<String>,
    pub weight: Option<f64>,
    pub enabled: Option<bool>,
}

/// `ccx manage edit NAME`: changes only the given fields. `--email` goes
/// only into `config.local.toml`. The other fields go into `config.toml`.
pub fn edit(parent: &Env, name: &str, edit: &Edit) -> Result<()> {
    if edit.email.is_none() && edit.weight.is_none() && edit.enabled.is_none() {
        return Err("nothing to change; give --email, --weight, --enable or --disable".into());
    }
    let setup = Setup::load(parent)?;
    setup.account(name)?;
    let mut main = adopt::read_doc(&setup.config_path)?;
    let mut overlay = adopt::read_doc(&setup.overlay_path)?;
    let (main_before, overlay_before) = (main.to_string(), overlay.to_string());
    let changes = apply_edit(&mut main, &mut overlay, name, edit)?;
    let (main_text, overlay_text) = add::validate(
        &main,
        &overlay,
        &setup.config_path,
        &setup.overlay_path,
        setup.home,
    )?;
    if main_text != main_before {
        let written =
            adopt::write_atomic(&setup.config_path, &main_text, mode_of(&setup.config_path))?;
        println!("Wrote {}", written.display());
    }
    if overlay_text != overlay_before {
        let written = adopt::write_atomic(&setup.overlay_path, &overlay_text, OVERLAY_MODE)?;
        println!("Wrote {} (mode 0600)", written.display());
    }
    for change in changes {
        println!("{name}: {change}");
    }
    Ok(())
}

/// Applies `edit` to the two documents. Returns one line for each change.
pub fn apply_edit(
    main: &mut DocumentMut,
    overlay: &mut DocumentMut,
    name: &str,
    edit: &Edit,
) -> Result<Vec<String>> {
    let mut changes = Vec::new();
    if let Some(weight) = edit.weight {
        if !(weight.is_finite() && weight > 0.0) {
            return Err(format!("--weight {weight}: must be a positive number"));
        }
        set_value(main, name, "weight", weight_value(weight))?;
        changes.push(format!("weight = {weight}"));
    }
    if let Some(enabled) = edit.enabled {
        set_value(main, name, "enabled", Value::from(enabled))?;
        changes.push(format!("enabled = {enabled}"));
    }
    if let Some(email) = &edit.email {
        if !valid_email(email) {
            return Err(format!("--email `{email}` is not a valid email address"));
        }
        adopt::set_email(overlay, name, email)?;
        changes.push(format!("email = {email} (in config.local.toml)"));
    }
    Ok(changes)
}

/// A simple check: one `@` with text on the two sides, and no spaces.
pub fn valid_email(email: &str) -> bool {
    match email.split_once('@') {
        Some((user, domain)) => {
            !user.is_empty()
                && !domain.is_empty()
                && !domain.contains('@')
                && !email.chars().any(char::is_whitespace)
        }
        None => false,
    }
}

/// A whole weight is written as an integer, for example `weight = 5`.
fn weight_value(weight: f64) -> Value {
    if weight.fract() == 0.0 && weight < 1e15 {
        Value::from(weight as i64)
    } else {
        Value::from(weight)
    }
}

/// Sets `accounts.NAME.KEY` in `config.toml`. A key that exists keeps its
/// position and its comments.
fn set_value(doc: &mut DocumentMut, name: &str, key: &str, mut value: Value) -> Result<()> {
    let table = doc
        .get_mut("accounts")
        .and_then(|accounts| accounts.get_mut(name))
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| format!("`accounts.{name}` is not a table in config.toml"))?;
    if let Some(old) = table.get(key).and_then(Item::as_value) {
        *value.decor_mut() = old.decor().clone();
    }
    match table.get_mut(key) {
        Some(item) => *item = Item::Value(value),
        None => {
            table.insert(key, Item::Value(value));
        }
    }
    Ok(())
}

/// `ccx manage edit-config`: opens a copy of `config.toml` in `$EDITOR`
/// (else `$VISUAL`, else `vi`). ccx replaces `config.toml` only when the
/// edited copy is valid.
pub fn edit_config(parent: &Env) -> Result<()> {
    let home = config::home(parent)?;
    let config_path = config::config_path(parent)?;
    let overlay_path = config::overlay_path(parent)?;
    let target = adopt::resolve_links(&config_path)?;
    let before = fs::read_to_string(&target)
        .map_err(|e| format!("cannot read {}: {e}", target.display()))?;
    let state = usage::state_dir(home);
    let copy = state.join("config.edit.toml");
    usage::make_private_dir(&state)
        .and_then(|()| fs::write(&copy, &before))
        .map_err(|e| format!("cannot write {}: {e}", copy.display()))?;

    let editor = ["EDITOR", "VISUAL"]
        .iter()
        .find_map(|key| parent.get(key))
        .unwrap_or(OsStr::new("vi"))
        .to_string_lossy()
        .into_owned();
    // The shell splits `$EDITOR`, so a value such as `code -w` works.
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(&copy)
        .env_clear()
        .envs(parent.iter())
        .status()
        .map_err(|e| format!("cannot run the editor `{editor}`: {e}"))?;
    let keep = |why: String| {
        format!(
            "{why}. {} did not change. Your edit is in {}",
            config_path.display(),
            copy.display()
        )
    };
    if !status.success() {
        return Err(keep(format!("the editor `{editor}` failed ({status})")));
    }
    let after =
        fs::read_to_string(&copy).map_err(|e| keep(format!("cannot read the edit: {e}")))?;
    if after == before {
        let _ = fs::remove_file(&copy);
        println!("No change.");
        return Ok(());
    }
    let overlay = match fs::read_to_string(&overlay_path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("cannot read {}: {e}", overlay_path.display())),
    };
    config::parse(
        &config::Source {
            path: &config_path,
            text: &after,
        },
        overlay
            .as_deref()
            .map(|text| config::Source {
                path: &overlay_path,
                text,
            })
            .as_ref(),
        home,
    )
    .map_err(|e| keep(format!("the edited config is not valid: {e}")))?;
    let written = adopt::write_atomic(&config_path, &after, mode_of(&target))?;
    let _ = fs::remove_file(&copy);
    println!("Wrote {}", written.display());
    Ok(())
}

/// The mode of the file at `path`, or the mode of a new `config.toml`.
fn mode_of(path: &Path) -> u32 {
    fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o7777)
        .unwrap_or(CONFIG_MODE)
}

/// `ccx manage rm NAME [--purge]`.
pub fn rm(parent: &Env, name: &str, purge: bool, yes: bool) -> Result<()> {
    let setup = Setup::load(parent)?;
    let account = setup.account(name)?;
    if account.is_base {
        return Err(format!(
            "`{name}` is the base account ({}); ccx manage rm does not remove it",
            account.dir.display()
        ));
    }
    let claude = if purge { Some(setup.claude()?) } else { None };
    let mut main = adopt::read_doc(&setup.config_path)?;
    let mut overlay = adopt::read_doc(&setup.overlay_path)?;
    remove_account(&mut main, name);
    let overlay_changed = remove_account(&mut overlay, name);
    let (main_text, overlay_text) = add::validate(
        &main,
        &overlay,
        &setup.config_path,
        &setup.overlay_path,
        setup.home,
    )?;

    let trash = trash_dir(setup.home);
    println!("ccx manage rm will:");
    for line in rm_plan(account, purge, &trash) {
        println!("  - {line}");
    }
    let retry = format!(
        "ccx manage rm {name}{} -y",
        if purge { " --purge" } else { "" }
    );
    if !adopt::confirm(yes, "Continue?", &retry)? {
        println!("Nothing changed.");
        return Ok(());
    }

    if let Some(claude) = &claude {
        match run_claude(claude, &setup.env(account)?, &["auth", "logout"]) {
            Ok(()) => println!("Logged out of account {name}."),
            Err(e) => eprintln!("ccx: warning: {e}; ccx continues with the purge"),
        }
        if delete_credentials(&account.dir)? {
            println!("Deleted the credentials file in {}", account.dir.display());
        }
        match move_to_trash(&account.dir, name, &trash)? {
            Some(to) => println!("Moved {} to {}", account.dir.display(), to.display()),
            None => println!("{} does not exist", account.dir.display()),
        }
    }

    let written = adopt::write_atomic(&setup.config_path, &main_text, mode_of(&setup.config_path))?;
    println!("Wrote {}", written.display());
    if overlay_changed {
        let written = adopt::write_atomic(&setup.overlay_path, &overlay_text, OVERLAY_MODE)?;
        println!("Wrote {} (mode 0600)", written.display());
    }
    if !purge {
        println!("The directory {} did not change.", account.dir.display());
    }
    let service = doctor::service_name(&account.dir);
    println!(
        "On macOS, the login of the account can stay in the keychain item `{service}`. ccx does not remove it. To remove it, run: security delete-generic-password -s '{service}'"
    );
    Ok(())
}

/// The plan lines that `rm` shows before it asks.
pub fn rm_plan(account: &Account, purge: bool, trash: &Path) -> Vec<String> {
    let name = &account.name;
    let dir = account.dir.display();
    let mut lines = Vec::new();
    if purge {
        lines.push(format!(
            "run `claude auth logout` with CLAUDE_CONFIG_DIR={dir}"
        ));
        lines.push(format!("delete {dir}/{CREDENTIALS_FILE} if it exists"));
        lines.push(format!(
            "move {dir} into {} (mode 0700); ccx never deletes the directory",
            trash.display()
        ));
    }
    lines.push(format!(
        "remove account {name} from config.toml and config.local.toml"
    ));
    if !purge {
        lines.push(format!("keep {dir}"));
    }
    lines
}

/// Removes `accounts.NAME`. Returns true when the document had it.
fn remove_account(doc: &mut DocumentMut, name: &str) -> bool {
    doc.get_mut("accounts")
        .and_then(Item::as_table_like_mut)
        .and_then(|accounts| accounts.remove(name))
        .is_some()
}

/// Deletes the credentials file in `dir`. It never reads the file. Returns
/// true when the file was there.
fn delete_credentials(dir: &Path) -> Result<bool> {
    match fs::remove_file(dir.join(CREDENTIALS_FILE)) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!(
            "cannot delete the credentials file in {}: {e}",
            dir.display()
        )),
    }
}

/// `~/.local/state/ccx/trash`.
pub fn trash_dir(home: &OsStr) -> PathBuf {
    usage::state_dir(home).join("trash")
}

/// Moves `dir` into `trash` as `NAME-<unix time>`. It makes `trash` at
/// mode 0700. Returns `None` when `dir` does not exist. It never deletes.
pub fn move_to_trash(dir: &Path, name: &str, trash: &Path) -> Result<Option<PathBuf>> {
    if fs::symlink_metadata(dir).is_err() {
        return Ok(None);
    }
    let error = |e: std::io::Error| format!("cannot make {}: {e}", trash.display());
    usage::make_private_dir(trash).map_err(error)?;
    fs::set_permissions(trash, fs::Permissions::from_mode(TRASH_MODE)).map_err(error)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    for n in 0..100 {
        let to = match n {
            0 => trash.join(format!("{name}-{stamp}")),
            n => trash.join(format!("{name}-{stamp}-{n}")),
        };
        if fs::symlink_metadata(&to).is_ok() {
            continue;
        }
        return match fs::rename(dir, &to) {
            Ok(()) => Ok(Some(to)),
            Err(e) => Err(format!(
                "cannot move {} to {}: {e}. The config did not change. Move the directory yourself, then run `ccx manage rm {name}`",
                dir.display(),
                to.display()
            )),
        };
    }
    Err(format!("cannot find a free name in {}", trash.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "\
# My accounts.
[accounts.main]
dir = \"~/.claude\"

[accounts.work]
dir = \"~/.claude-work\" # the work dir
weight = 2 # small
enabled = true

[pick]
stickiness = 5
";

    fn docs() -> (DocumentMut, DocumentMut) {
        (CONFIG.parse().unwrap(), DocumentMut::new())
    }

    #[test]
    fn edit_changes_only_the_given_fields_and_keeps_comments() {
        let (mut main, mut overlay) = docs();
        let edit = Edit {
            weight: Some(5.0),
            enabled: Some(false),
            ..Edit::default()
        };
        let changes = apply_edit(&mut main, &mut overlay, "work", &edit).unwrap();
        assert_eq!(changes, ["weight = 5", "enabled = false"]);
        assert_eq!(
            main.to_string(),
            CONFIG
                .replace("weight = 2 # small", "weight = 5 # small")
                .replace("enabled = true", "enabled = false")
        );
        assert_eq!(overlay.to_string(), "");
    }

    #[test]
    fn edit_adds_a_missing_key_and_writes_a_fraction_as_a_float() {
        let (mut main, mut overlay) = docs();
        let edit = Edit {
            weight: Some(2.5),
            ..Edit::default()
        };
        apply_edit(&mut main, &mut overlay, "main", &edit).unwrap();
        assert!(
            main.to_string()
                .contains("[accounts.main]\ndir = \"~/.claude\"\nweight = 2.5\n"),
            "{main}"
        );
    }

    #[test]
    fn edit_email_goes_only_to_the_overlay() {
        let (mut main, mut overlay) = docs();
        let edit = Edit {
            email: Some("work@example.com".into()),
            ..Edit::default()
        };
        apply_edit(&mut main, &mut overlay, "work", &edit).unwrap();
        assert_eq!(main.to_string(), CONFIG);
        assert_eq!(
            overlay.to_string(),
            "[accounts.work]\nemail = \"work@example.com\"\n"
        );
    }

    #[test]
    fn edit_rejects_values_that_are_not_valid() {
        for weight in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let (mut main, mut overlay) = docs();
            let edit = Edit {
                weight: Some(weight),
                ..Edit::default()
            };
            let e = apply_edit(&mut main, &mut overlay, "work", &edit).unwrap_err();
            assert!(e.contains("positive number"), "{e}");
        }
        for email in [
            "",
            "work",
            "@example.com",
            "work@",
            "a b@example.com",
            "a@b@c",
        ] {
            let (mut main, mut overlay) = docs();
            let edit = Edit {
                email: Some(email.into()),
                ..Edit::default()
            };
            assert!(
                apply_edit(&mut main, &mut overlay, "work", &edit).is_err(),
                "{email}"
            );
        }
        assert!(valid_email("work@example.com"));
    }

    #[test]
    fn no_browser_in_ssh_or_on_linux_with_no_display() {
        let env = |pairs: &[(&str, &str)]| pairs.iter().copied().collect::<Env>();
        assert!(no_browser(
            &env(&[("SSH_CONNECTION", "192.0.2.1 1 192.0.2.2 22")]),
            false
        ));
        assert!(no_browser(&env(&[("SSH_TTY", "/dev/ttys001")]), false));
        assert!(!no_browser(&env(&[]), false));
        assert!(no_browser(&env(&[]), true));
        assert!(no_browser(&env(&[("DISPLAY", "")]), true));
        assert!(!no_browser(&env(&[("DISPLAY", ":0")]), true));
        assert!(!no_browser(&env(&[("WAYLAND_DISPLAY", "wayland-0")]), true));
    }

    #[test]
    fn check_email_compares_without_case() {
        assert!(check_email("w", "W@Example.com", Some("w@example.com")).is_ok());
        let e = check_email("w", "w@example.com", Some("x@example.com")).unwrap_err();
        assert!(
            e.contains("x@example.com") && e.contains("w@example.com"),
            "{e}"
        );
        assert!(e.contains("ccx manage logout w"), "{e}");
        assert!(check_email("w", "w@example.com", None).is_err());
    }

    #[test]
    fn remove_account_reports_if_it_was_there() {
        let (mut main, mut overlay) = docs();
        assert!(remove_account(&mut main, "work"));
        assert!(!main.to_string().contains("work"), "{main}");
        assert!(
            main.to_string()
                .starts_with("# My accounts.\n[accounts.main]")
        );
        assert!(!remove_account(&mut overlay, "work"));
    }

    #[test]
    fn rm_plan_names_each_step() {
        let account = Account {
            name: "work".into(),
            dir: PathBuf::from("/home/test/.claude-work"),
            dir_text: "~/.claude-work".into(),
            weight: None,
            aliases: Vec::new(),
            enabled: true,
            email: None,
            own: Vec::new(),
            is_base: false,
        };
        let trash = Path::new("/home/test/.local/state/ccx/trash");
        let text = rm_plan(&account, false, trash).join("\n");
        assert!(text.contains("keep /home/test/.claude-work"), "{text}");
        assert!(!text.contains("logout"), "{text}");
        let text = rm_plan(&account, true, trash).join("\n");
        assert!(text.contains("claude auth logout"), "{text}");
        assert!(text.contains("never deletes the directory"), "{text}");
        assert!(!text.contains("keep "), "{text}");
    }

    #[test]
    fn move_to_trash_makes_a_private_trash_and_never_overwrites() {
        let root = tempfile::tempdir().unwrap();
        let trash = root.path().join("state/trash");
        assert_eq!(
            move_to_trash(&root.path().join("missing"), "x", &trash).unwrap(),
            None
        );
        let mut moved = Vec::new();
        for _ in 0..2 {
            let dir = root.path().join(".claude-x");
            fs::create_dir(&dir).unwrap();
            fs::write(dir.join("file"), "kept").unwrap();
            let to = move_to_trash(&dir, "x", &trash).unwrap().unwrap();
            assert!(!dir.exists());
            assert_eq!(fs::read_to_string(to.join("file")).unwrap(), "kept");
            moved.push(to);
        }
        assert_ne!(moved[0], moved[1]);
        let mode = fs::metadata(&trash).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
}

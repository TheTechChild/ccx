//! `ccx manage adopt`: find the Claude Code accounts on this machine and
//! write them into `config.toml` and `config.local.toml`.
//!
//! Adopt writes nothing until the user confirms. `config.toml` gets the
//! account dirs and never an email. The emails go only into the overlay.

use std::fs;
use std::io::{IsTerminal, Write as _};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use toml_edit::{DocumentMut, Item, Table, value};

use crate::config::{self, Account, BASE_DIR, Result, Source};
use crate::env::Env;
use crate::launch;
use crate::paths::canonical;

/// The name that adopt gives to the base account `~/.claude`.
pub const BASE_NAME: &str = "main";

/// Files in `~/.claude` that show that the base account was once started
/// with `CLAUDE_CONFIG_DIR=~/.claude`. Claude Code keeps the real file of
/// the base account at `~/.claude.json`.
pub const STRAY: [&str; 2] = [".claude.json", ".claude.json.lock"];

/// The mode of `config.local.toml`.
const OVERLAY_MODE: u32 = 0o600;
/// The mode of a new `config.toml`.
const CONFIG_MODE: u32 = 0o644;

/// An account dir that adopt found in HOME.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The proposed account name.
    pub name: String,
    /// The dir as `config.toml` shows it, for example `~/.claude-angel2`.
    pub dir_text: String,
}

/// Runs `ccx manage adopt`. `yes` confirms without a prompt.
pub fn run(parent: &Env, yes: bool) -> Result<()> {
    let home = PathBuf::from(config::home(parent)?);
    let config_path = config::config_path(parent)?;
    let overlay_path = config::overlay_path(parent)?;
    let mut main = read_doc(&config_path)?;
    let mut overlay = read_doc(&overlay_path)?;
    let (found, mut notes) = discover(&home)?;
    if !found.iter().any(|f| f.dir_text == BASE_DIR) {
        return Err(format!(
            "cannot find the base account {}",
            home.join(".claude").display()
        ));
    }

    let claude_bin = main
        .get("launch")
        .and_then(|launch| launch.get("claude_bin"))
        .and_then(Item::as_str)
        .map(|bin| canonical(bin, home.as_os_str()));
    let self_exe = std::env::current_exe().ok();
    let claude = launch::find_claude(claude_bin.as_deref(), parent, self_exe.as_deref());
    if claude.is_none() {
        notes.push("cannot find claude, so adopt cannot read the emails".into());
    }
    let depth = launch::check_depth(parent)?;
    let base_dir = canonical(BASE_DIR, home.as_os_str());

    let mut plan = Vec::new();
    let (mut main_changed, mut overlay_changed) = (false, false);
    for found in &found {
        let dir = canonical(&found.dir_text, home.as_os_str());
        let name = match name_for_dir(&main, &dir, &home) {
            Some(name) => name,
            None if has_account(&main, &found.name) => {
                notes.push(format!(
                    "skip {}: another dir already uses the name `{}`",
                    found.dir_text, found.name
                ));
                continue;
            }
            None => {
                add_account(&mut main, &found.name, &found.dir_text)?;
                main_changed = true;
                plan.push(format!("add account {} = {}", found.name, found.dir_text));
                found.name.clone()
            }
        };
        let account = Account {
            name: name.clone(),
            is_base: dir == base_dir,
            dir,
            dir_text: found.dir_text.clone(),
            weight: None,
            aliases: Vec::new(),
            enabled: true,
            email: None,
            own: Vec::new(),
        };
        let Some(claude) = &claude else { continue };
        match auth_email(claude, parent, &account, depth) {
            Some(email) if email_of(&overlay, &name) != Some(email.as_str()) => {
                set_email(&mut overlay, &name, &email)?;
                overlay_changed = true;
                plan.push(format!("set the email of {name} to {email}"));
            }
            Some(_) => {}
            None => notes.push(format!(
                "no email for {name}: `claude auth status` shows no login"
            )),
        }
    }
    let stray = find_stray(&home);
    for path in &stray {
        plan.push(format!("move {} to a backup directory", path.display()));
    }

    for note in &notes {
        println!("note: {note}");
    }
    if plan.is_empty() {
        println!("Nothing to adopt. The config is up to date.");
        return Ok(());
    }
    println!("ccx manage adopt will:");
    for line in &plan {
        println!("  - {line}");
    }
    if main_changed {
        println!("  - write {}", config_path.display());
    }
    if overlay_changed {
        println!("  - write {} (mode 0600)", overlay_path.display());
    }

    // The new files must load with the normal loader before adopt writes them.
    let (main_text, overlay_text) = (main.to_string(), overlay.to_string());
    config::parse(
        &Source {
            path: &config_path,
            text: &main_text,
        },
        Some(&Source {
            path: &overlay_path,
            text: &overlay_text,
        }),
        home.as_os_str(),
    )?;

    if !confirm(yes)? {
        println!("Nothing written.");
        return Ok(());
    }
    if main_changed {
        let mode = fs::metadata(&config_path)
            .map(|m| m.permissions().mode() & 0o7777)
            .unwrap_or(CONFIG_MODE);
        let written = write_atomic(&config_path, &main_text, mode)?;
        println!("Wrote {}", written.display());
    }
    if overlay_changed {
        let written = write_atomic(&overlay_path, &overlay_text, OVERLAY_MODE)?;
        println!("Wrote {}", written.display());
    }
    if !stray.is_empty() {
        let backup = move_to_backup(&stray, &backup_root(parent, &home))?;
        println!("Moved the stray files to {}", backup.display());
    }
    Ok(())
}

/// Finds `~/.claude` and each `~/.claude-*` dir. Returns the accounts (the
/// base account first, then by name) and notes about the dirs it skipped.
pub fn discover(home: &Path) -> Result<(Vec<Found>, Vec<String>)> {
    let entries = fs::read_dir(home).map_err(|e| format!("cannot read {}: {e}", home.display()))?;
    let mut found = Vec::new();
    let mut notes = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read {}: {e}", home.display()))?;
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let name = match file_name.strip_prefix(".claude-") {
            Some(suffix) => suffix,
            None if file_name == ".claude" => BASE_NAME,
            None => continue,
        };
        if !entry.path().is_dir() {
            continue;
        }
        if !config::valid_name(name) {
            notes.push(format!(
                "skip ~/{file_name}: `{name}` is not a valid account name"
            ));
            continue;
        }
        found.push(Found {
            name: name.to_string(),
            dir_text: format!("~/{file_name}"),
        });
    }
    found.sort_by(|a, b| {
        let key = |f: &Found| (f.dir_text != BASE_DIR, f.name.clone());
        key(a).cmp(&key(b))
    });
    Ok((found, notes))
}

/// Reads a TOML file for editing. A missing file gives an empty document.
fn read_doc(path: &Path) -> Result<DocumentMut> {
    match fs::read_to_string(path) {
        Ok(text) => text.parse().map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// The name of the account in `doc` whose dir is `dir` after expansion.
fn name_for_dir(doc: &DocumentMut, dir: &Path, home: &Path) -> Option<String> {
    let accounts = doc.get("accounts")?.as_table_like()?;
    accounts.iter().find_map(|(name, account)| {
        let text = account.get("dir")?.as_str()?;
        (canonical(text, home.as_os_str()) == dir).then(|| name.to_string())
    })
}

fn has_account(doc: &DocumentMut, name: &str) -> bool {
    doc.get("accounts")
        .and_then(|accounts| accounts.get(name))
        .is_some()
}

fn email_of<'a>(doc: &'a DocumentMut, name: &str) -> Option<&'a str> {
    doc.get("accounts")?.get(name)?.get("email")?.as_str()
}

/// The `[accounts]` table. It is implicit, so it has no header of its own.
fn accounts_table(doc: &mut DocumentMut) -> Result<&mut Table> {
    doc.entry("accounts")
        .or_insert_with(|| {
            let mut table = Table::new();
            table.set_implicit(true);
            Item::Table(table)
        })
        .as_table_mut()
        .ok_or_else(|| "`accounts` must be a table".to_string())
}

fn add_account(doc: &mut DocumentMut, name: &str, dir_text: &str) -> Result<()> {
    let mut table = Table::new();
    table.insert("dir", value(dir_text));
    accounts_table(doc)?.insert(name, Item::Table(table));
    Ok(())
}

fn set_email(doc: &mut DocumentMut, name: &str, email: &str) -> Result<()> {
    accounts_table(doc)?
        .entry(name)
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| format!("`accounts.{name}` must be a table"))?
        .insert("email", value(email));
    Ok(())
}

/// Runs `claude auth status --json` as `account` and returns the email.
/// The environment is the launch environment, so `CLAUDE_CONFIG_DIR` is
/// not set for the base account.
fn auth_email(claude: &Path, parent: &Env, account: &Account, depth: u32) -> Option<String> {
    let (env, _) = launch::build_env(parent, account, depth);
    let out = Command::new(claude)
        .args(["auth", "status", "--json"])
        .env_clear()
        .envs(env.iter())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    parse_email(&out.stdout)
}

/// Gets the email from the output of `claude auth status --json`.
fn parse_email(stdout: &[u8]) -> Option<String> {
    let status: serde_json::Value = serde_json::from_slice(stdout).ok()?;
    if status["loggedIn"] != true {
        return None;
    }
    status["email"]
        .as_str()
        .filter(|email| !email.is_empty())
        .map(str::to_string)
}

fn confirm(yes: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        return Err(
            "no terminal to confirm the changes. Nothing was written. To confirm, run `ccx manage adopt -y`"
                .into(),
        );
    }
    dialoguer::Confirm::new()
        .with_prompt("Write these changes?")
        .default(false)
        .interact()
        .map_err(|e| format!("cannot read the answer: {e}"))
}

/// Follows symlinks to the file that a write must replace. The target can
/// be missing. Only the last part of the path must be a symlink.
pub fn resolve_links(path: &Path) -> Result<PathBuf> {
    let mut path = path.to_path_buf();
    for _ in 0..40 {
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let target = fs::read_link(&path)
                    .map_err(|e| format!("cannot read the link {}: {e}", path.display()))?;
                // An absolute target replaces the whole path.
                path = path.parent().unwrap_or(Path::new("/")).join(target);
            }
            _ => return Ok(path),
        }
    }
    Err(format!("too many symlinks at {}", path.display()))
}

/// Writes `text` to a temp file next to the target and renames it into
/// place. A symlink at `path` stays: the write replaces its target.
/// Returns the path of the file that it wrote.
pub fn write_atomic(path: &Path, text: &str, mode: u32) -> Result<PathBuf> {
    let target = resolve_links(path)?;
    let dir = match target.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let error = |e: std::io::Error| format!("cannot write {}: {e}", target.display());
    fs::create_dir_all(dir).map_err(error)?;
    let name = target
        .file_name()
        .ok_or_else(|| format!("{} is not a file path", target.display()))?;
    let tmp = dir.join(format!(
        ".{}.ccx-{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&tmp)?;
        // The umask can remove bits from `mode`. Set the mode again.
        file.set_permissions(fs::Permissions::from_mode(mode))?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, &target)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(error(e));
    }
    Ok(target)
}

/// The stray files in `~/.claude` that exist.
fn find_stray(home: &Path) -> Vec<PathBuf> {
    STRAY
        .iter()
        .map(|name| home.join(".claude").join(name))
        .filter(|path| fs::symlink_metadata(path).is_ok())
        .collect()
}

/// `$XDG_STATE_HOME/ccx/backup`, else `~/.local/state/ccx/backup`.
pub fn backup_root(env: &Env, home: &Path) -> PathBuf {
    match env.get("XDG_STATE_HOME").map(Path::new) {
        Some(state) if state.is_absolute() => state.join("ccx/backup"),
        _ => home.join(".local/state/ccx/backup"),
    }
}

/// Moves each path into a new dir `adopt-<unix time>` in `root` and
/// returns that dir. It never deletes a file.
pub fn move_to_backup(paths: &[PathBuf], root: &Path) -> Result<PathBuf> {
    let error = |e: std::io::Error| format!("cannot make a backup in {}: {e}", root.display());
    fs::create_dir_all(root).map_err(error)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut backup = None;
    for n in 0..100 {
        let dir = match n {
            0 => root.join(format!("adopt-{stamp}")),
            n => root.join(format!("adopt-{stamp}-{n}")),
        };
        match fs::create_dir(&dir) {
            Ok(()) => {
                backup = Some(dir);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(error(e)),
        }
    }
    let backup = backup.ok_or_else(|| format!("cannot make a backup dir in {}", root.display()))?;
    for path in paths {
        let name = path.file_name().unwrap_or(path.as_os_str());
        fs::rename(path, backup.join(name))
            .map_err(|e| format!("cannot move {}: {e}", path.display()))?;
    }
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(name: &str, dir_text: &str) -> Found {
        Found {
            name: name.into(),
            dir_text: dir_text.into(),
        }
    }

    #[test]
    fn discover_finds_the_base_and_each_claude_dash_dir() {
        let home = tempfile::tempdir().unwrap();
        for dir in [
            ".claude-zeta",
            ".claude",
            ".claude-angel2",
            ".claude-Bad",
            ".other",
        ] {
            fs::create_dir(home.path().join(dir)).unwrap();
        }
        // Files are not accounts.
        fs::write(home.path().join(".claude.json"), "{}").unwrap();
        fs::write(home.path().join(".claude-file"), "").unwrap();
        // A symlink to a dir is an account dir.
        std::os::unix::fs::symlink(
            home.path().join(".claude-zeta"),
            home.path().join(".claude-link"),
        )
        .unwrap();

        let (found_dirs, notes) = discover(home.path()).unwrap();
        assert_eq!(
            found_dirs,
            [
                found("main", "~/.claude"),
                found("angel2", "~/.claude-angel2"),
                found("link", "~/.claude-link"),
                found("zeta", "~/.claude-zeta"),
            ]
        );
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("~/.claude-Bad"), "{notes:?}");
    }

    #[test]
    fn edits_keep_comments_and_other_entries() {
        let text = "# My accounts.\n[launch]\nbanner = false # quiet\n\n[accounts.main]\ndir = \"~/.claude\"\n";
        let mut doc: DocumentMut = text.parse().unwrap();
        let home = Path::new("/home/test");
        assert_eq!(
            name_for_dir(&doc, Path::new("/home/test/.claude"), home).as_deref(),
            Some("main")
        );
        assert!(has_account(&doc, "main"));
        add_account(&mut doc, "angel2", "~/.claude-angel2").unwrap();
        assert_eq!(
            doc.to_string(),
            format!("{text}\n[accounts.angel2]\ndir = \"~/.claude-angel2\"\n")
        );
    }

    #[test]
    fn new_docs_have_no_empty_accounts_header() {
        let mut doc = DocumentMut::new();
        set_email(&mut doc, "main", "main@example.com").unwrap();
        assert_eq!(email_of(&doc, "main"), Some("main@example.com"));
        assert_eq!(
            doc.to_string(),
            "[accounts.main]\nemail = \"main@example.com\"\n"
        );
    }

    #[test]
    fn inline_accounts_value_is_an_error() {
        let mut doc: DocumentMut = "accounts = 1\n".parse().unwrap();
        assert!(add_account(&mut doc, "a", "~/.a").is_err());
    }

    #[test]
    fn parse_email_needs_a_login() {
        let ok = br#"{"loggedIn": true, "email": "a@example.com"}"#;
        assert_eq!(parse_email(ok).as_deref(), Some("a@example.com"));
        for bad in [
            &br#"{"loggedIn": false, "email": "a@example.com"}"#[..],
            br#"{"loggedIn": true}"#,
            br#"{"loggedIn": true, "email": ""}"#,
            b"not json",
        ] {
            assert_eq!(parse_email(bad), None);
        }
    }

    #[test]
    fn write_atomic_replaces_the_symlink_target_and_keeps_the_link() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let target = repo.join("config.toml");
        fs::write(&target, "old").unwrap();
        let link = dir.path().join("config.toml");
        std::os::unix::fs::symlink("repo/config.toml", &link).unwrap();

        let written = write_atomic(&link, "new", 0o644).unwrap();
        assert_eq!(written, target);
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        // No temp file stays.
        assert_eq!(fs::read_dir(&repo).unwrap().count(), 1);
    }

    #[test]
    fn write_atomic_sets_the_mode_and_makes_the_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/config.local.toml");
        write_atomic(&path, "x", 0o600).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        // A dangling link gets its target made.
        let link = dir.path().join("link.toml");
        std::os::unix::fs::symlink(dir.path().join("missing.toml"), &link).unwrap();
        write_atomic(&link, "y", 0o644).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("missing.toml")).unwrap(),
            "y"
        );
    }

    #[test]
    fn move_to_backup_never_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".claude.json");
        let lock = dir.path().join(".claude.json.lock");
        fs::write(&file, "{}").unwrap();
        fs::create_dir(&lock).unwrap();
        let root = dir.path().join("backup");
        let first = move_to_backup(&[file.clone(), lock.clone()], &root).unwrap();
        assert!(!file.exists() && !lock.exists());
        assert_eq!(
            fs::read_to_string(first.join(".claude.json")).unwrap(),
            "{}"
        );
        assert!(first.join(".claude.json.lock").is_dir());

        // A second backup in the same second gets a new dir.
        fs::write(&file, "{}").unwrap();
        let second = move_to_backup(&[file], &root).unwrap();
        assert_ne!(first, second);
        assert!(first.join(".claude.json").exists());
    }

    #[test]
    fn backup_root_uses_xdg_state_home() {
        let home = Path::new("/home/test");
        let env: Env = [("XDG_STATE_HOME", "/state")].into_iter().collect();
        assert_eq!(backup_root(&env, home), PathBuf::from("/state/ccx/backup"));
        let env = Env::default();
        assert_eq!(
            backup_root(&env, home),
            PathBuf::from("/home/test/.local/state/ccx/backup")
        );
    }
}

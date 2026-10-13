//! `ccx manage sync`: share the setup of the base account with the other
//! accounts through symlinks. Refer to ADR 0003 and ADR 0008.
//!
//! Safety rules:
//! - Sync never deletes through a symlink. RELINK removes only the link.
//! - A CONFLICT item moves to `<dir>/.ccx-backup/<timestamp>/`. Sync never
//!   removes a real file or directory.
//! - Sync refuses an operation whose resolved path is inside the base
//!   directory or outside the account directory.
//! - Sync never changes the base account.

use std::fs;
use std::io::{ErrorKind, IsTerminal};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{self, Account, Config, Result};
use crate::env::Env;

/// Top-level items that sync links as one item.
pub const SHARED: [&str; 8] = [
    "settings.json",
    "CLAUDE.md",
    "keybindings.json",
    "statusline.py",
    "agents",
    "commands",
    "output-styles",
    "plugins",
];

/// The skills directory. It is a real directory in each account, with one
/// link for each child of the base `skills/`.
pub const SKILLS: &str = "skills";

/// The child of `skills/` that sync never links: it is keyed per
/// organization and account.
pub const SKILLS_SYNCED: &str = "synced";

/// Top-level items that sync never shares. All dotfiles are also in this
/// group.
pub const NEVER: [&str; 22] = [
    "projects",
    "history.jsonl",
    "file-history",
    "plans",
    "todos",
    "sessions",
    "session-env",
    "shell-snapshots",
    "backups",
    "cache",
    "ide",
    "daemon",
    "telemetry",
    "paste-cache",
    "jobs",
    "state",
    "settings.local.json",
    "remote-settings.json",
    "mcp-needs-auth-cache.json",
    // Dotfiles, also listed here so that the list is complete.
    ".claude.json",
    ".credentials.json",
    ".claude.json.lock",
];

/// The backup directory in each account.
pub const BACKUP_DIR: &str = ".ccx-backup";

/// The group of a top-level entry of the base directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Shared,
    Skills,
    Never,
    Unknown,
}

/// Puts a top-level entry of the base directory into its group.
pub fn classify(name: &str) -> Class {
    if SHARED.contains(&name) {
        Class::Shared
    } else if name == SKILLS {
        Class::Skills
    } else if name.starts_with('.') || NEVER.contains(&name) {
        Class::Never
    } else {
        Class::Unknown
    }
}

/// True when `item` can be in `own = [...]`: a shared item, `skills`, or
/// `skills/<name>`.
pub fn valid_own(item: &str) -> bool {
    SHARED.contains(&item)
        || item == SKILLS
        || item.strip_prefix("skills/").is_some_and(|name| {
            !name.is_empty() && !name.contains('/') && !matches!(name, "." | ".." | SKILLS_SYNCED)
        })
}

/// What the account must have at a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Want {
    /// A symlink with this absolute target.
    Link(PathBuf),
    /// A real directory.
    Dir,
}

/// The `lstat` result for a path in the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Missing,
    /// A symlink, with the text of its target.
    Link(PathBuf),
    File,
    Dir,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// The path is correct.
    Ok,
    /// Make a missing link.
    Link,
    /// Replace a link that points to a different target. Only the link is
    /// removed.
    Relink,
    /// A real file or directory is where a link (or a directory) must be.
    /// It moves to the backup directory.
    Conflict,
    /// The account keeps its own item (`own = [...]`).
    Skip,
    /// Make the real `skills/` directory. A link at that path is removed
    /// first.
    Mkdir,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Ok => "OK",
            Action::Link => "LINK",
            Action::Relink => "RELINK",
            Action::Conflict => "CONFLICT",
            Action::Skip => "SKIP",
            Action::Mkdir => "MKDIR",
        }
    }
}

/// Decides the action for one path.
pub fn decide(want: &Want, state: &State, owned: bool) -> Action {
    if owned {
        return Action::Skip;
    }
    match (want, state) {
        (Want::Link(_), State::Missing) => Action::Link,
        (Want::Link(target), State::Link(current)) if current == target => Action::Ok,
        (Want::Link(_), State::Link(_)) => Action::Relink,
        (Want::Link(_), State::File | State::Dir) => Action::Conflict,
        (Want::Dir, State::Dir) => Action::Ok,
        (Want::Dir, State::Missing | State::Link(_)) => Action::Mkdir,
        (Want::Dir, State::File) => Action::Conflict,
    }
}

/// One path in the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The path relative to the account directory.
    pub path: PathBuf,
    pub want: Want,
    pub state: State,
    pub action: Action,
}

impl Item {
    fn is_skill(&self) -> bool {
        self.path.parent() == Some(Path::new(SKILLS))
    }
}

/// The sync plan for one account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub base: PathBuf,
    pub account: PathBuf,
    /// Top-level base entries in no group. Sync does not share them.
    pub unknown: Vec<String>,
    /// Shared items, then `skills`, then one item for each base skill.
    pub items: Vec<Item>,
}

/// Reads the state of `path` with `lstat`.
pub fn lstat(path: &Path) -> Result<State> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(State::Missing),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
        Ok(meta) if meta.file_type().is_symlink() => fs::read_link(path)
            .map(State::Link)
            .map_err(|e| format!("cannot read the link {}: {e}", path.display())),
        Ok(meta) if meta.is_dir() => Ok(State::Dir),
        Ok(_) => Ok(State::File),
    }
}

/// The names in `dir`, sorted.
pub fn names(dir: &Path) -> Result<Vec<std::ffi::OsString>> {
    let error = |e: std::io::Error| format!("cannot read {}: {e}", dir.display());
    let mut names = fs::read_dir(dir)
        .map_err(error)?
        .map(|entry| entry.map(|e| e.file_name()))
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(error)?;
    names.sort();
    Ok(names)
}

/// Makes the plan for the account dir `account`. `base` is the base
/// directory. Link targets are `base` joined with the item name, so they are
/// absolute when `base` is absolute. The planner only reads.
pub fn plan(base: &Path, account: &Path, own: &[String]) -> Result<Plan> {
    if !base.is_dir() {
        return Err(format!("the base directory {} is missing", base.display()));
    }
    if !account.is_dir() {
        return Err(format!(
            "the account directory {} is missing",
            account.display()
        ));
    }
    let owned = |item: &str| own.iter().any(|o| o == item);
    let mut unknown = Vec::new();
    let mut items = Vec::new();
    let mut has_skills = false;
    for name in names(base)? {
        let text = name.to_string_lossy();
        match classify(&text) {
            Class::Shared => {
                let want = Want::Link(base.join(&name));
                let state = lstat(&account.join(&name))?;
                let action = decide(&want, &state, owned(&text));
                items.push(Item {
                    path: PathBuf::from(&name),
                    want,
                    state,
                    action,
                });
            }
            Class::Skills => has_skills = base.join(SKILLS).is_dir(),
            Class::Never => {}
            Class::Unknown => unknown.push(text.into_owned()),
        }
    }
    if has_skills {
        let state = lstat(&account.join(SKILLS))?;
        let action = decide(&Want::Dir, &state, owned(SKILLS));
        let skills_is_dir = state == State::Dir;
        items.push(Item {
            path: PathBuf::from(SKILLS),
            want: Want::Dir,
            state,
            action,
        });
        if action != Action::Skip {
            for name in names(&base.join(SKILLS))? {
                let text = name.to_string_lossy();
                if text == SKILLS_SYNCED || text.starts_with('.') {
                    continue;
                }
                let path = Path::new(SKILLS).join(&name);
                let want = Want::Link(base.join(&path));
                // A `skills` path that is not a real directory gets a new
                // directory, so each skill starts as missing. Never read
                // through a `skills` link.
                let state = match skills_is_dir {
                    true => lstat(&account.join(&path))?,
                    false => State::Missing,
                };
                let action = decide(&want, &state, owned(&format!("{SKILLS}/{text}")));
                items.push(Item {
                    path,
                    want,
                    state,
                    action,
                });
            }
        }
    }
    Ok(Plan {
        base: base.to_path_buf(),
        account: account.to_path_buf(),
        unknown,
        items,
    })
}

/// The real form of `path`: the real path of the nearest parent that
/// exists, then the other names. The last name is not followed, because an
/// operation changes the link itself, not its target.
fn resolved(path: &Path) -> Result<PathBuf> {
    let bad = || format!("cannot resolve {}", path.display());
    let mut rest = vec![path.file_name().ok_or_else(bad)?];
    let mut dir = path.parent().ok_or_else(bad)?;
    loop {
        match fs::canonicalize(dir) {
            Ok(mut real) => {
                real.extend(rest.iter().rev());
                return Ok(real);
            }
            // Only a path that does not exist at all is OK here. A dangling
            // link has an unknown target.
            Err(e) if e.kind() == ErrorKind::NotFound && fs::symlink_metadata(dir).is_err() => {
                rest.push(dir.file_name().ok_or_else(bad)?);
                dir = dir.parent().ok_or_else(bad)?;
            }
            Err(e) => return Err(format!("cannot resolve {}: {e}", dir.display())),
        }
    }
}

/// Refuses operations outside the account directory or inside the base
/// directory, after symlinks are resolved.
pub struct Guard {
    base: PathBuf,
    account: PathBuf,
}

impl Guard {
    pub fn new(base: &Path, account: &Path) -> Result<Guard> {
        let real = |p: &Path| {
            fs::canonicalize(p).map_err(|e| format!("cannot resolve {}: {e}", p.display()))
        };
        Ok(Guard {
            base: real(base)?,
            account: real(account)?,
        })
    }

    pub fn check(&self, path: &Path) -> Result<()> {
        let real = resolved(path)?;
        let problem = if real.starts_with(&self.base) {
            "inside the base directory"
        } else if real == self.account || !real.starts_with(&self.account) {
            "outside the account directory"
        } else {
            return Ok(());
        };
        Err(format!(
            "refuse to change {}: its real path {} is {problem}",
            path.display(),
            real.display()
        ))
    }
}

/// Checks the path of one operation before any change. When `skills` is
/// not a real directory yet, sync makes a new one, so a skill path is
/// checked as a name in that new directory, not through the old path.
fn check_item(guard: &Guard, plan: &Plan, item: &Item) -> Result<()> {
    let skills_is_dir = plan
        .items
        .iter()
        .any(|i| i.path == Path::new(SKILLS) && i.state == State::Dir);
    if item.is_skill() && !skills_is_dir {
        guard.check(&plan.account.join(SKILLS))
    } else {
        guard.check(&plan.account.join(&item.path))
    }
}

/// Which actions `apply` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// LINK, RELINK and MKDIR. CONFLICT only when `conflicts` is true.
    Full { conflicts: bool },
    /// The fast check of `ccx @name`: only LINK, and MKDIR of a missing
    /// `skills/`. It never changes a link or a conflict.
    Health,
}

fn selected(item: &Item, mode: Mode) -> bool {
    match (mode, item.action) {
        (_, Action::Ok | Action::Skip) => false,
        (Mode::Full { conflicts }, Action::Conflict) => conflicts,
        (Mode::Full { .. }, _) => true,
        (Mode::Health, Action::Link) => true,
        (Mode::Health, Action::Mkdir) => item.state == State::Missing,
        (Mode::Health, _) => false,
    }
}

/// The items that `apply` changes in `mode`. A skill is in the list only
/// when `skills/` is a real directory or becomes one.
pub fn operations(plan: &Plan, mode: Mode) -> Vec<&Item> {
    let skills_ready = plan.items.iter().any(|item| {
        item.path == Path::new(SKILLS) && (item.action == Action::Ok || selected(item, mode))
    });
    plan.items
        .iter()
        .filter(|item| selected(item, mode) && (!item.is_skill() || skills_ready))
        .collect()
}

/// The result of `apply`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Applied {
    pub changed: usize,
    /// The backup directory, if a CONFLICT item moved.
    pub backup: Option<PathBuf>,
}

/// Does the operations of `plan` in `mode`. It checks all paths with the
/// guard before it changes a file.
pub fn apply(plan: &Plan, mode: Mode) -> Result<Applied> {
    let ops = operations(plan, mode);
    if ops.is_empty() {
        return Ok(Applied::default());
    }
    let guard = Guard::new(&plan.base, &plan.account)?;
    let backup_root = plan.account.join(BACKUP_DIR);
    for item in &ops {
        check_item(&guard, plan, item)?;
        if item.action == Action::Conflict {
            guard.check(&backup_root.join(&item.path))?;
        }
    }

    let mut applied = Applied::default();
    for item in ops {
        let path = plan.account.join(&item.path);
        let error = |e: std::io::Error| format!("cannot change {}: {e}", path.display());
        // Check again: an earlier operation can change a parent directory.
        guard.check(&path)?;
        // The state must not change between the plan and the change.
        let now = lstat(&path)?;
        if now != item.state {
            return Err(format!(
                "{} changed after the plan; run sync again",
                path.display()
            ));
        }
        match (item.action, &item.state) {
            (Action::Conflict, _) => {
                let dir = match &applied.backup {
                    Some(dir) => dir.clone(),
                    None => {
                        let dir = new_backup_dir(&backup_root, &guard)?;
                        applied.backup = Some(dir.clone());
                        dir
                    }
                };
                let dest = dir.join(&item.path);
                guard.check(&dest)?;
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).map_err(error)?;
                }
                fs::rename(&path, &dest).map_err(error)?;
            }
            // Remove only the link. `remove_file` never follows it.
            (_, State::Link(_)) => fs::remove_file(&path).map_err(error)?,
            _ => {}
        }
        match &item.want {
            Want::Link(target) => symlink(target, &path).map_err(error)?,
            Want::Dir => fs::create_dir(&path).map_err(error)?,
        }
        applied.changed += 1;
    }
    Ok(applied)
}

/// Makes a new directory `<root>/<timestamp>` and returns it.
fn new_backup_dir(root: &Path, guard: &Guard) -> Result<PathBuf> {
    let error = |e: std::io::Error| format!("cannot make a backup in {}: {e}", root.display());
    guard.check(root)?;
    fs::create_dir_all(root).map_err(error)?;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let stamp = timestamp(secs);
    for n in 0..100 {
        let dir = match n {
            0 => root.join(&stamp),
            n => root.join(format!("{stamp}-{n}")),
        };
        guard.check(&dir)?;
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(error(e)),
        }
    }
    Err(format!(
        "cannot make a new backup directory in {}",
        root.display()
    ))
}

/// Formats Unix seconds as `YYYYMMDDTHHMMSSZ` (UTC).
pub fn timestamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    // Civil date from the day number (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

/// One line of the plan output.
fn describe(item: &Item) -> String {
    let path = item.path.display();
    let detail = match (item.action, &item.want, &item.state) {
        (Action::Skip, _, _) => " (own)".to_string(),
        (Action::Conflict, _, State::Dir) => {
            format!(" (real directory; moves to {BACKUP_DIR}/)")
        }
        (Action::Conflict, _, _) => format!(" (real file; moves to {BACKUP_DIR}/)"),
        (Action::Relink, Want::Link(target), State::Link(old)) => {
            format!(" -> {} (now -> {})", target.display(), old.display())
        }
        (Action::Mkdir, _, State::Link(old)) => {
            format!(" (replaces the link to {})", old.display())
        }
        (Action::Link, Want::Link(target), _) => format!(" -> {}", target.display()),
        _ => String::new(),
    };
    format!("  {:<9} {path}{detail}", item.action.label())
}

/// The plan as text.
pub fn render(name: &str, plan: &Plan) -> String {
    let mut out = format!("{name} ({}):\n", plan.account.display());
    for item in &plan.items {
        out.push_str(&describe(item));
        out.push('\n');
    }
    out
}

/// The unknown base entries as text.
pub fn render_unknown(unknown: &[String]) -> String {
    unknown
        .iter()
        .map(|name| format!("  {:<9} {name} (not shared)\n", "UNKNOWN"))
        .collect()
}

/// The fast check of `ccx @name`: adds missing links and never changes a
/// conflict. Returns the number of new links and directories.
pub fn health_check(base: &Account, account: &Account) -> Result<usize> {
    if account.is_base {
        return Ok(0);
    }
    let plan = plan(&base.dir, &account.dir, &account.own)?;
    Ok(apply(&plan, Mode::Health)?.changed)
}

/// The accounts that `ccx manage sync` changes.
pub fn targets<'a>(config: &'a Config, name: Option<&str>) -> Result<Vec<&'a Account>> {
    let Some(name) = name else {
        return Ok(config.accounts.iter().filter(|a| !a.is_base).collect());
    };
    let name = name.strip_prefix('@').unwrap_or(name);
    let account = config.account(name).ok_or_else(|| {
        format!(
            "unknown account `{name}`; known accounts: {}",
            config.names().join(", ")
        )
    })?;
    if account.is_base {
        return Err(format!(
            "`{name}` is the base account; sync never changes the base account"
        ));
    }
    Ok(vec![account])
}

/// Runs `ccx manage sync`. `name` is `None` for `--all`. Returns false when
/// CONFLICT items stay unchanged.
pub fn run(env: &Env, name: Option<&str>, dry_run: bool, force: bool) -> Result<bool> {
    Ok(run_counted(env, name, dry_run, force)?.ok)
}

/// The result of [`run_counted`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Synced {
    /// False when CONFLICT items stay unchanged.
    pub ok: bool,
    /// The number of links and directories that sync made or changed.
    pub changed: usize,
}

/// Runs `ccx manage sync` and returns the number of changes too.
pub fn run_counted(env: &Env, name: Option<&str>, dry_run: bool, force: bool) -> Result<Synced> {
    let config = config::load(env)?;
    let base = config.base();
    let accounts = targets(&config, name)?;
    if accounts.is_empty() {
        println!("No accounts to sync. Only the base account is in the config.");
        return Ok(Synced {
            ok: true,
            changed: 0,
        });
    }

    let mut plans = Vec::new();
    for account in accounts {
        let plan = plan(&base.dir, &account.dir, &account.own)?;
        // Check the guard also in a dry run, so that the user sees a refusal.
        let guard = Guard::new(&plan.base, &plan.account)?;
        for item in operations(&plan, Mode::Full { conflicts: true }) {
            check_item(&guard, &plan, item)?;
        }
        plans.push((account, plan));
    }

    if let Some((_, first)) = plans.first()
        && !first.unknown.is_empty()
    {
        print!(
            "base ({}):\n{}",
            base.dir.display(),
            render_unknown(&first.unknown)
        );
    }
    for (account, plan) in &plans {
        print!("{}", render(&account.name, plan));
    }
    if dry_run {
        println!("Dry run. Nothing changed.");
        return Ok(Synced {
            ok: true,
            changed: 0,
        });
    }

    let conflicts: usize = plans
        .iter()
        .map(|(_, plan)| {
            plan.items
                .iter()
                .filter(|item| item.action == Action::Conflict)
                .count()
        })
        .sum();
    let apply_conflicts = conflicts > 0 && (force || confirm(conflicts)?);

    let mut changed = 0;
    for (account, plan) in &plans {
        let applied = apply(
            plan,
            Mode::Full {
                conflicts: apply_conflicts,
            },
        )?;
        changed += applied.changed;
        if let Some(dir) = applied.backup {
            println!(
                "{}: moved the CONFLICT items to {}",
                account.name,
                dir.display()
            );
        }
    }
    println!("Sync made {changed} changes.");
    if conflicts > 0 && !apply_conflicts {
        println!(
            "{conflicts} CONFLICT items did not change. To move them to {BACKUP_DIR}/ and link them, run sync with --force."
        );
    }
    Ok(Synced {
        ok: conflicts == 0 || apply_conflicts,
        changed,
    })
}

/// Asks to move the CONFLICT items. With no terminal, the answer is no.
fn confirm(conflicts: usize) -> Result<bool> {
    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        return Ok(false);
    }
    dialoguer::Confirm::new()
        .with_prompt(format!(
            "Move {conflicts} CONFLICT items to {BACKUP_DIR}/ and link them?"
        ))
        .default(false)
        .interact()
        .map_err(|e| format!("cannot read the answer: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp dir with `base` and `acct` in it.
    struct Tree {
        root: tempfile::TempDir,
    }

    impl Tree {
        fn new() -> Tree {
            let root = tempfile::tempdir().unwrap();
            fs::create_dir(root.path().join("base")).unwrap();
            fs::create_dir(root.path().join("acct")).unwrap();
            Tree { root }
        }
        fn base(&self) -> PathBuf {
            self.root.path().join("base")
        }
        fn acct(&self) -> PathBuf {
            self.root.path().join("acct")
        }
        fn plan(&self, own: &[&str]) -> Plan {
            let own: Vec<String> = own.iter().map(|s| s.to_string()).collect();
            plan(&self.base(), &self.acct(), &own).unwrap()
        }
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn item<'a>(plan: &'a Plan, path: &str) -> &'a Item {
        plan.items
            .iter()
            .find(|i| i.path == Path::new(path))
            .unwrap_or_else(|| panic!("no item {path} in {plan:#?}"))
    }

    fn action(plan: &Plan, path: &str) -> Action {
        item(plan, path).action
    }

    #[test]
    fn classify_each_group() {
        for name in SHARED {
            assert_eq!(classify(name), Class::Shared, "{name}");
        }
        assert_eq!(classify("skills"), Class::Skills);
        for name in NEVER {
            assert_eq!(classify(name), Class::Never, "{name}");
        }
        for name in [".git", ".DS_Store", ".claude.json.lock", ".anything"] {
            assert_eq!(classify(name), Class::Never, "{name}");
        }
        for name in ["chrome", "downloads", "README.md", "settings.json.bak"] {
            assert_eq!(classify(name), Class::Unknown, "{name}");
        }
    }

    #[test]
    fn own_values() {
        for good in ["settings.json", "plugins", "skills", "skills/tdd"] {
            assert!(valid_own(good), "{good}");
        }
        for bad in [
            "projects",
            "chrome",
            "skills/",
            "skills/synced",
            "skills/a/b",
            "skills/..",
            "",
        ] {
            assert!(!valid_own(bad), "{bad}");
        }
    }

    #[test]
    fn decide_table() {
        let target = Want::Link(PathBuf::from("/b/x"));
        let link = |t: &str| State::Link(PathBuf::from(t));
        let cases = [
            (&target, State::Missing, false, Action::Link),
            (&target, link("/b/x"), false, Action::Ok),
            (&target, link("/elsewhere/x"), false, Action::Relink),
            (&target, link("../b/x"), false, Action::Relink),
            (&target, State::File, false, Action::Conflict),
            (&target, State::Dir, false, Action::Conflict),
            (&target, State::File, true, Action::Skip),
            (&target, State::Missing, true, Action::Skip),
            (&Want::Dir, State::Dir, false, Action::Ok),
            (&Want::Dir, State::Missing, false, Action::Mkdir),
            (&Want::Dir, link("/b/skills"), false, Action::Mkdir),
            (&Want::Dir, State::File, false, Action::Conflict),
            (&Want::Dir, State::Dir, true, Action::Skip),
        ];
        for (want, state, owned, expected) in cases {
            assert_eq!(decide(want, &state, owned), expected, "{want:?} {state:?}");
        }
    }

    #[test]
    fn planner_covers_each_lstat_result() {
        let t = Tree::new();
        let (base, acct) = (t.base(), t.acct());
        for name in [
            "settings.json",
            "CLAUDE.md",
            "keybindings.json",
            "statusline.py",
        ] {
            write(&base.join(name), name);
        }
        for name in ["agents", "commands", "plugins"] {
            fs::create_dir(base.join(name)).unwrap();
        }
        // missing: CLAUDE.md
        symlink(base.join("settings.json"), acct.join("settings.json")).unwrap();
        symlink("/elsewhere/plugins", acct.join("plugins")).unwrap();
        write(&acct.join("keybindings.json"), "mine");
        fs::create_dir(acct.join("agents")).unwrap();
        write(&acct.join("statusline.py"), "own copy");
        fs::create_dir(acct.join("commands")).unwrap();

        let plan = t.plan(&["statusline.py", "commands"]);
        assert_eq!(action(&plan, "CLAUDE.md"), Action::Link);
        assert_eq!(action(&plan, "settings.json"), Action::Ok);
        assert_eq!(action(&plan, "plugins"), Action::Relink);
        assert_eq!(action(&plan, "keybindings.json"), Action::Conflict);
        assert_eq!(item(&plan, "keybindings.json").state, State::File);
        assert_eq!(action(&plan, "agents"), Action::Conflict);
        assert_eq!(item(&plan, "agents").state, State::Dir);
        assert_eq!(action(&plan, "statusline.py"), Action::Skip);
        assert_eq!(action(&plan, "commands"), Action::Skip);
        assert_eq!(
            item(&plan, "CLAUDE.md").want,
            Want::Link(base.join("CLAUDE.md"))
        );
        // Items that the base does not have are not in the plan.
        assert!(
            plan.items
                .iter()
                .all(|i| i.path != Path::new("output-styles"))
        );
    }

    #[test]
    fn planner_classifies_every_base_entry() {
        let t = Tree::new();
        let base = t.base();
        for name in [
            "settings.json",
            "history.jsonl",
            ".claude.json",
            "README.md",
            "daemon.log",
        ] {
            write(&base.join(name), "x");
        }
        for name in ["projects", "chrome", ".git", "skills"] {
            fs::create_dir(base.join(name)).unwrap();
        }
        let plan = t.plan(&[]);
        assert_eq!(plan.unknown, ["README.md", "chrome", "daemon.log"]);
        let paths: Vec<_> = plan.items.iter().map(|i| i.path.clone()).collect();
        assert_eq!(
            paths,
            [PathBuf::from("settings.json"), PathBuf::from("skills")]
        );
    }

    #[test]
    fn planner_links_each_skill_but_never_synced() {
        let t = Tree::new();
        let (base, acct) = (t.base(), t.acct());
        for name in ["tdd", "deploy", "synced", "local"] {
            fs::create_dir_all(base.join("skills").join(name)).unwrap();
        }
        write(&base.join("skills/.DS_Store"), "x");
        fs::create_dir_all(acct.join("skills/synced")).unwrap();
        fs::create_dir_all(acct.join("skills/mine")).unwrap();
        symlink(base.join("skills/tdd"), acct.join("skills/tdd")).unwrap();
        fs::create_dir(acct.join("skills/local")).unwrap();

        let plan = t.plan(&["skills/local"]);
        assert_eq!(action(&plan, "skills"), Action::Ok);
        assert_eq!(action(&plan, "skills/tdd"), Action::Ok);
        assert_eq!(action(&plan, "skills/deploy"), Action::Link);
        assert_eq!(action(&plan, "skills/local"), Action::Skip);
        let paths: Vec<_> = plan.items.iter().map(|i| i.path.clone()).collect();
        assert!(!paths.contains(&PathBuf::from("skills/synced")));
        assert!(!paths.contains(&PathBuf::from("skills/.DS_Store")));
        // A local-only skill is not in the plan.
        assert!(!paths.contains(&PathBuf::from("skills/mine")));
    }

    #[test]
    fn planner_never_reads_through_a_skills_link() {
        let t = Tree::new();
        let (base, acct) = (t.base(), t.acct());
        fs::create_dir_all(base.join("skills/tdd")).unwrap();
        symlink(base.join("skills"), acct.join("skills")).unwrap();
        let plan = t.plan(&[]);
        assert_eq!(action(&plan, "skills"), Action::Mkdir);
        // Through the link, tdd is a real dir. The plan does not look.
        assert_eq!(action(&plan, "skills/tdd"), Action::Link);
        assert_eq!(item(&plan, "skills/tdd").state, State::Missing);
    }

    #[test]
    fn owned_skills_dir_skips_all_skills() {
        let t = Tree::new();
        fs::create_dir_all(t.base().join("skills/tdd")).unwrap();
        let plan = t.plan(&["skills"]);
        assert_eq!(action(&plan, "skills"), Action::Skip);
        assert_eq!(plan.items.len(), 1);
    }

    #[test]
    fn planner_reports_missing_dirs() {
        let t = Tree::new();
        let missing = t.root.path().join("nope");
        assert!(
            plan(&t.base(), &missing, &[])
                .unwrap_err()
                .contains("account directory")
        );
        assert!(
            plan(&missing, &t.acct(), &[])
                .unwrap_err()
                .contains("base directory")
        );
    }

    #[test]
    fn guard_refuses_paths_in_base_or_outside_the_account() {
        let t = Tree::new();
        let (base, acct) = (t.base(), t.acct());
        let other = t.root.path().join("other");
        fs::create_dir_all(base.join("skills")).unwrap();
        fs::create_dir(&other).unwrap();
        symlink(base.join("skills"), acct.join("skills")).unwrap();
        symlink(&other, acct.join("out")).unwrap();
        let guard = Guard::new(&base, &acct).unwrap();

        guard.check(&acct.join("settings.json")).unwrap();
        guard.check(&acct.join("new/deeper/name")).unwrap();
        // The link itself is in the account, so it can change.
        guard.check(&acct.join("skills")).unwrap();

        let err = guard.check(&acct.join("skills/tdd")).unwrap_err();
        assert!(err.contains("inside the base directory"), "{err}");
        let err = guard.check(&acct.join("out/x")).unwrap_err();
        assert!(err.contains("outside the account directory"), "{err}");
        let err = guard.check(&acct.join("../acct")).unwrap_err();
        assert!(err.contains("outside the account directory"), "{err}");
        // A path that ends in `..` has no name. It is refused too.
        assert!(guard.check(&acct.join("..")).is_err());
    }

    #[test]
    fn guard_refuses_an_account_that_is_a_link_to_base() {
        let t = Tree::new();
        let alias = t.root.path().join("alias");
        symlink(t.base(), &alias).unwrap();
        write(&t.base().join("settings.json"), "base");
        let guard = Guard::new(&t.base(), &alias).unwrap();
        let err = guard.check(&alias.join("settings.json")).unwrap_err();
        assert!(err.contains("inside the base directory"), "{err}");

        // apply refuses before it changes a file.
        let plan = plan(&t.base(), &alias, &[]).unwrap();
        assert_eq!(action(&plan, "settings.json"), Action::Conflict);
        let err = apply(&plan, Mode::Full { conflicts: true }).unwrap_err();
        assert!(err.contains("refuse"), "{err}");
        assert_eq!(
            fs::read_to_string(t.base().join("settings.json")).unwrap(),
            "base"
        );
        assert!(!t.base().join(BACKUP_DIR).exists());
    }

    #[test]
    fn apply_moves_conflicts_to_a_backup_and_relinks_only_the_link() {
        let t = Tree::new();
        let (base, acct) = (t.base(), t.acct());
        write(&base.join("settings.json"), "base settings");
        fs::create_dir(base.join("agents")).unwrap();
        fs::create_dir(base.join("plugins")).unwrap();
        fs::create_dir_all(base.join("skills/tdd")).unwrap();
        write(&acct.join("settings.json"), "account settings");
        write(&acct.join("agents/a.md"), "agent");
        let other = t.root.path().join("other-plugins");
        write(&other.join("keep.txt"), "keep");
        symlink(&other, acct.join("plugins")).unwrap();
        fs::create_dir_all(acct.join("skills")).unwrap();
        write(&acct.join("skills/tdd/SKILL.md"), "local tdd");

        let plan = t.plan(&[]);
        let applied = apply(&plan, Mode::Full { conflicts: true }).unwrap();
        assert_eq!(applied.changed, 4);
        let backup = applied.backup.unwrap();
        assert_eq!(backup.parent().unwrap(), acct.join(BACKUP_DIR));

        assert_eq!(
            fs::read_to_string(backup.join("settings.json")).unwrap(),
            "account settings"
        );
        assert_eq!(
            fs::read_to_string(backup.join("agents/a.md")).unwrap(),
            "agent"
        );
        assert_eq!(
            fs::read_to_string(backup.join("skills/tdd/SKILL.md")).unwrap(),
            "local tdd"
        );
        for name in ["settings.json", "agents", "plugins", "skills/tdd"] {
            assert_eq!(
                fs::read_link(acct.join(name)).unwrap(),
                base.join(name),
                "{name}"
            );
        }
        // RELINK removed only the link. The old target is intact.
        assert_eq!(fs::read_to_string(other.join("keep.txt")).unwrap(), "keep");
        assert_eq!(
            fs::read_to_string(base.join("settings.json")).unwrap(),
            "base settings"
        );

        // A second run has nothing to do.
        let again = t.plan(&[]);
        assert!(
            again.items.iter().all(|i| i.action == Action::Ok),
            "{again:#?}"
        );
    }

    #[test]
    fn apply_without_conflicts_leaves_them() {
        let t = Tree::new();
        write(&t.base().join("settings.json"), "base");
        write(&t.base().join("CLAUDE.md"), "base");
        write(&t.acct().join("settings.json"), "mine");
        let applied = apply(&t.plan(&[]), Mode::Full { conflicts: false }).unwrap();
        assert_eq!(applied.changed, 1);
        assert_eq!(applied.backup, None);
        assert_eq!(
            fs::read_to_string(t.acct().join("settings.json")).unwrap(),
            "mine"
        );
        assert!(!t.acct().join(BACKUP_DIR).exists());
    }

    #[test]
    fn skills_link_becomes_a_real_dir_and_base_skills_stay() {
        let t = Tree::new();
        let (base, acct) = (t.base(), t.acct());
        write(&base.join("skills/tdd/SKILL.md"), "tdd");
        symlink(base.join("skills"), acct.join("skills")).unwrap();
        apply(&t.plan(&[]), Mode::Full { conflicts: false }).unwrap();
        assert_eq!(lstat(&acct.join("skills")).unwrap(), State::Dir);
        assert_eq!(
            fs::read_link(acct.join("skills/tdd")).unwrap(),
            base.join("skills/tdd")
        );
        assert_eq!(
            fs::read_to_string(base.join("skills/tdd/SKILL.md")).unwrap(),
            "tdd"
        );
        assert_eq!(names(&base.join("skills")).unwrap(), ["tdd"]);
    }

    #[test]
    fn health_mode_adds_missing_links_only() {
        let t = Tree::new();
        let (base, acct) = (t.base(), t.acct());
        write(&base.join("settings.json"), "base");
        write(&base.join("CLAUDE.md"), "base");
        fs::create_dir(base.join("plugins")).unwrap();
        fs::create_dir_all(base.join("skills/new-skill")).unwrap();
        write(&acct.join("settings.json"), "mine");
        symlink("/elsewhere", acct.join("plugins")).unwrap();

        let applied = apply(&t.plan(&[]), Mode::Health).unwrap();
        // CLAUDE.md, skills/ and skills/new-skill.
        assert_eq!(applied.changed, 3);
        assert_eq!(
            fs::read_to_string(acct.join("settings.json")).unwrap(),
            "mine"
        );
        assert_eq!(
            fs::read_link(acct.join("plugins")).unwrap(),
            Path::new("/elsewhere")
        );
        assert_eq!(
            fs::read_link(acct.join("skills/new-skill")).unwrap(),
            base.join("skills/new-skill")
        );
    }

    #[test]
    fn health_mode_skips_skills_when_skills_is_a_link() {
        let t = Tree::new();
        fs::create_dir_all(t.base().join("skills/tdd")).unwrap();
        symlink(t.base().join("skills"), t.acct().join("skills")).unwrap();
        let applied = apply(&t.plan(&[]), Mode::Health).unwrap();
        assert_eq!(applied.changed, 0);
        assert!(lstat(&t.acct().join("skills")).unwrap() != State::Dir);
    }

    #[test]
    fn apply_stops_when_the_tree_changed_after_the_plan() {
        let t = Tree::new();
        write(&t.base().join("CLAUDE.md"), "base");
        let plan = t.plan(&[]);
        write(&t.acct().join("CLAUDE.md"), "new");
        let err = apply(&plan, Mode::Full { conflicts: true }).unwrap_err();
        assert!(err.contains("changed after the plan"), "{err}");
        assert_eq!(
            fs::read_to_string(t.acct().join("CLAUDE.md")).unwrap(),
            "new"
        );
    }

    #[test]
    fn timestamp_golden() {
        assert_eq!(timestamp(0), "19700101T000000Z");
        assert_eq!(timestamp(951_868_799), "20000229T235959Z");
        assert_eq!(timestamp(1_791_854_786), "20261013T012626Z");
    }

    #[test]
    fn render_shows_each_action() {
        let t = Tree::new();
        let (base, acct) = (t.base(), t.acct());
        write(&base.join("CLAUDE.md"), "x");
        write(&base.join("settings.json"), "x");
        write(&acct.join("settings.json"), "x");
        let text = render("angel2", &t.plan(&[]));
        assert!(
            text.starts_with(&format!("angel2 ({}):\n", acct.display())),
            "{text}"
        );
        assert!(text.contains(&format!(
            "  LINK      CLAUDE.md -> {}",
            base.join("CLAUDE.md").display()
        )));
        assert!(text.contains("  CONFLICT  settings.json (real file; moves to .ccx-backup/)"));
        assert_eq!(
            render_unknown(&["chrome".into()]),
            "  UNKNOWN   chrome (not shared)\n"
        );
    }
}

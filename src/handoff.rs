//! `ccx handoff`: copy a session to a different account, then resume it
//! there (#14).
//!
//! Handoff copies. It never moves, deletes or changes a file of the source
//! account. The file set and the cwd encoding come from M0 test 4
//! (ADR 0006). The fork method is in ADR 0009.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, IsTerminal, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::config::{self, Account, Config, Result};
use crate::env::Env;
use crate::{launch, paths, pick, usage};

/// Handoff refuses when the source transcript changed less than this time
/// ago, because the session is then probably open.
pub const RECENT: Duration = Duration::from_secs(5);

/// The maximum length of an encoded cwd before the hash (ADR 0006).
const MAX_NAME: usize = 200;

/// The list shows this number of characters of the first prompt.
const PROMPT_WIDTH: usize = 60;

/// The summary reads no more than this number of transcript lines.
const MAX_SUMMARY_LINES: usize = 2000;

/// The exit code when ccx cannot start claude.
const EXIT_EXEC_FAILED: u8 = 127;

/// The command-line arguments of `ccx handoff`.
pub struct Args {
    pub id: Option<String>,
    pub target: Option<String>,
    pub all: bool,
    pub no_resume: bool,
}

/// One session transcript in one account.
#[derive(Debug, Clone)]
pub struct Thread {
    pub id: String,
    pub account: String,
    /// The transcript: `projects/<enc>/<id>.jsonl`.
    pub path: PathBuf,
    pub modified: SystemTime,
}

/// The data that the list shows from a transcript.
#[derive(Debug, Default, PartialEq)]
pub struct Summary {
    pub prompt: Option<String>,
    pub cwd: Option<String>,
}

/// How handoff writes the transcript into the target account.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// The target has no copy.
    New,
    /// The target copy is a prefix of the source: overwrite it.
    Overwrite,
    /// The target copy is not a prefix of the source: write a fork with a
    /// new session id.
    Fork,
}

/// The result of a handoff.
#[derive(Debug)]
pub struct Done {
    /// The session id in the target account.
    pub id: String,
    pub plan: Plan,
    /// The number of files that handoff wrote.
    pub files: usize,
    /// The `cwd` field of the transcript.
    pub cwd: Option<PathBuf>,
}

/// Makes the `projects/` directory name for `cwd`, as Claude Code does
/// (ADR 0006). `cwd` must be the physical path, with no symlinks.
pub fn encode_cwd(cwd: &str) -> String {
    let name: String = cwd
        .encode_utf16()
        .map(|unit| match u8::try_from(unit) {
            Ok(b) if b.is_ascii_alphanumeric() => char::from(b),
            _ => '-',
        })
        .collect();
    if name.len() <= MAX_NAME {
        return name;
    }
    let hash = java_hash(cwd).unsigned_abs();
    format!("{}-{}", &name[..MAX_NAME], base36(u64::from(hash)))
}

/// The Java string hash over the UTF-16 code units of `text`.
fn java_hash(text: &str) -> i32 {
    text.encode_utf16().fold(0i32, |h, unit| {
        h.wrapping_mul(31).wrapping_add(i32::from(unit))
    })
}

fn base36(mut n: u64) -> String {
    let mut digits = Vec::new();
    loop {
        digits.push(char::from_digit((n % 36) as u32, 36).expect("a digit below 36"));
        n /= 36;
        if n == 0 {
            break;
        }
    }
    digits.iter().rev().collect()
}

/// True for a session id: hex digits and `-` only. This also makes sure
/// that an id from the command line cannot name a different path.
pub fn is_session_id(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// The transcripts of `account`: only in `projects/<enc>/` when `enc` is
/// given, else in all `projects/` directories.
pub fn transcripts(account: &Account, enc: Option<&str>) -> Vec<Thread> {
    let projects = account.dir.join("projects");
    let dirs = match enc {
        Some(enc) => vec![projects.join(enc)],
        None => child_dirs(&projects),
    };
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(id) = name.to_str().and_then(|n| n.strip_suffix(".jsonl")) else {
                continue;
            };
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !is_session_id(id) || !meta.is_file() {
                continue;
            }
            out.push(Thread {
                id: id.to_string(),
                account: account.name.clone(),
                path: entry.path(),
                modified: meta.modified().unwrap_or(UNIX_EPOCH),
            });
        }
    }
    out
}

/// The child directories of `dir`, sorted by name.
fn child_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

/// The threads of all accounts, the newest first.
pub fn list(config: &Config, enc: Option<&str>) -> Vec<Thread> {
    let mut threads: Vec<Thread> = config
        .accounts
        .iter()
        .flat_map(|account| transcripts(account, enc))
        .collect();
    threads.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.id.cmp(&b.id)));
    threads
}

/// The transcript `projects/*/<id>.jsonl` of session `id` in all accounts.
/// When more than one account has it, the newest copy wins. On a tie, the
/// first account in name order wins.
pub fn find_by_id(config: &Config, id: &str) -> Option<Thread> {
    if !is_session_id(id) {
        return None;
    }
    let file = format!("{id}.jsonl");
    let file = file.as_str();
    config
        .accounts
        .iter()
        .flat_map(|account| {
            child_dirs(&account.dir.join("projects"))
                .into_iter()
                .filter_map(move |dir| {
                    let path = dir.join(file);
                    let meta = fs::metadata(&path).ok().filter(|m| m.is_file())?;
                    Some(Thread {
                        id: id.to_string(),
                        account: account.name.clone(),
                        path,
                        modified: meta.modified().unwrap_or(UNIX_EPOCH),
                    })
                })
        })
        .min_by(|a, b| b.modified.cmp(&a.modified))
}

/// Reads the first user prompt and the `cwd` field from the start of a
/// transcript.
pub fn summary(reader: impl BufRead) -> Summary {
    let mut out = Summary::default();
    for line in reader.lines().take(MAX_SUMMARY_LINES) {
        let Ok(line) = line else {
            break;
        };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if out.cwd.is_none() {
            out.cwd = value.get("cwd").and_then(Value::as_str).map(String::from);
        }
        if out.prompt.is_none() {
            out.prompt = user_prompt(&value);
        }
        if out.cwd.is_some() && out.prompt.is_some() {
            break;
        }
    }
    out
}

/// The text of a user prompt line. Meta lines, tool results and command
/// wrappers (text that starts with `<`) are not prompts.
fn user_prompt(line: &Value) -> Option<String> {
    if line.get("type")?.as_str()? != "user" || line.get("isMeta") == Some(&Value::Bool(true)) {
        return None;
    }
    let content = line.pointer("/message/content")?;
    let text = match content {
        Value::String(text) => text.as_str(),
        Value::Array(parts) => parts
            .iter()
            .find(|p| p.get("type").and_then(Value::as_str) == Some("text"))?
            .get("text")?
            .as_str()?,
        _ => return None,
    };
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty() && !text.starts_with('<')).then_some(text)
}

fn read_summary(path: &Path) -> Summary {
    fs::File::open(path)
        .map(|f| summary(BufReader::new(f)))
        .unwrap_or_default()
}

/// Shortens `text` to `width` characters, with `…` at the end when it is
/// longer.
fn shorten(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width - 1).collect();
    out.push('…');
    out
}

/// `2026-10-12 14:05` in local time.
fn time_text(time: SystemTime) -> String {
    let ms = time
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    match pick::local_tm(ms) {
        Some(tm) => format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min
        ),
        None => "?".into(),
    }
}

/// One line for each thread. With `all`, the lines also show the
/// directory of the thread.
fn rows(threads: &[Thread], all: bool, home: &OsStr) -> Vec<String> {
    let width = threads.iter().map(|t| t.account.len()).max().unwrap_or(0);
    threads
        .iter()
        .map(|t| {
            let summary = read_summary(&t.path);
            let prompt = shorten(summary.prompt.as_deref().unwrap_or("-"), PROMPT_WIDTH);
            let mut line = format!(
                "{}  {:width$}  {}  ",
                t.id,
                t.account,
                time_text(t.modified)
            );
            if all {
                let dir = summary
                    .cwd
                    .map_or("-".into(), |c| paths::tilde(Path::new(&c), home));
                line.push_str(&dir);
                line.push_str("  ");
            }
            line.push_str(&prompt);
            line
        })
        .collect()
}

/// The distinct `slug` values of a transcript, in order of their first use.
pub fn slugs(transcript: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for line in transcript.lines().filter(|l| l.contains("\"slug\"")) {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(slug) = value.get("slug").and_then(Value::as_str)
            && is_slug(slug)
            && seen.insert(slug.to_string())
        {
            out.push(slug.to_string());
        }
    }
    out
}

/// A slug is a file name part: letters, digits, `-` and `_`.
fn is_slug(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The plan files in `dir` for the slugs: `plans/<slug>.md`, if present.
pub fn plan_files(dir: &Path, slugs: &[String]) -> Vec<PathBuf> {
    slugs
        .iter()
        .map(|slug| Path::new("plans").join(format!("{slug}.md")))
        .filter(|rel| dir.join(rel).is_file())
        .collect()
}

/// The rule for a target that has a copy already (re-handoff).
pub fn plan(target: Option<&[u8]>, source: &[u8]) -> Plan {
    match target {
        None => Plan::New,
        Some(target) if source.starts_with(target) => Plan::Overwrite,
        Some(_) => Plan::Fork,
    }
}

/// The directories of session `id` in `dir`, relative to `dir`, with the
/// path for the id `new` in the target: each `projects/*/<id>/` sidecar,
/// `file-history/<id>/` and `session-env/<id>/`.
pub fn session_dirs(dir: &Path, id: &str, new: &str) -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    for project in child_dirs(&dir.join("projects")) {
        let name = project.file_name().expect("a child has a name");
        let rel = Path::new("projects").join(name);
        if project.join(id).is_dir() {
            out.push((rel.join(id), rel.join(new)));
        }
    }
    for top in ["file-history", "session-env"] {
        if dir.join(top).join(id).is_dir() {
            out.push((Path::new(top).join(id), Path::new(top).join(new)));
        }
    }
    out
}

/// True when the name is a `sessions/<pid>.json` file. The `.key` files
/// in the same directory never agree, so ccx never reads them.
pub fn is_session_file(name: &str) -> bool {
    name.strip_suffix(".json")
        .is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()))
}

/// The pid of an open session `id` in the account directory `dir`, from
/// the `sessions/<pid>.json` files (ADR 0006).
pub fn open_pid(dir: &Path, id: &str) -> Option<u32> {
    let entries = fs::read_dir(dir.join("sessions")).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_str().is_some_and(is_session_file) {
            continue;
        }
        let Ok(text) = fs::read(entry.path()) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<Value>(&text) else {
            continue;
        };
        if value.get("sessionId").and_then(Value::as_str) != Some(id) {
            continue;
        }
        let Some(pid) = value
            .get("pid")
            .and_then(Value::as_u64)
            .and_then(|p| u32::try_from(p).ok())
        else {
            continue;
        };
        let proc_start = value.get("procStart").and_then(Value::as_str);
        if is_running(pid, proc_start) {
            return Some(pid);
        }
    }
    None
}

/// True when `pid` is a live process and its start time agrees with
/// `proc_start`. When ccx cannot get the start time, it uses only the
/// pid check.
fn is_running(pid: u32, proc_start: Option<&str>) -> bool {
    let Ok(pid_t) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid_t <= 0 {
        return false;
    }
    // SAFETY: signal 0 sends nothing. It only checks the pid.
    let alive = unsafe { libc::kill(pid_t, 0) } == 0
        || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    if !alive {
        return false;
    }
    match (proc_start, process_start(pid)) {
        (Some(want), Some(have)) => same_words(want, &have),
        _ => true,
    }
}

/// The start time of `pid` in the `ps -o lstart=` format, in UTC.
fn process_start(pid: u32) -> Option<String> {
    let out = Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .env("TZ", "UTC")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let text = text.trim();
    (out.status.success() && !text.is_empty()).then(|| text.to_string())
}

/// Compares two texts and ignores the number of spaces between words.
fn same_words(a: &str, b: &str) -> bool {
    a.split_whitespace().eq(b.split_whitespace())
}

/// A new random session id (UUID version 4).
fn new_session_id() -> Result<String> {
    let mut b = [0u8; 16];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .map_err(|e| format!("cannot make a new session id: {e}"))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

/// Changes the `sessionId` field of each line from `old` to `new`.
/// Claude Code writes compact JSON, and text in a JSON string has escaped
/// quotes, so only the field itself agrees with the pattern.
pub fn rewrite_session_id(text: &str, old: &str, new: &str) -> String {
    text.replace(
        &format!("\"sessionId\":\"{old}\""),
        &format!("\"sessionId\":\"{new}\""),
    )
}

/// Copies a file or a directory tree. With `rewrite`, each `.jsonl` file
/// gets the new session id. Symlinks are not copied. Returns the number of
/// files written.
fn copy_tree(from: &Path, to: &Path, rewrite: Option<(&str, &str)>) -> Result<usize> {
    let error = |e: std::io::Error| format!("cannot copy {}: {e}", from.display());
    let meta = fs::symlink_metadata(from).map_err(error)?;
    if meta.is_dir() {
        fs::create_dir_all(to).map_err(|e| format!("cannot make {}: {e}", to.display()))?;
        let mut names: Vec<_> = fs::read_dir(from)
            .map_err(error)?
            .flatten()
            .map(|e| e.file_name())
            .collect();
        names.sort();
        let mut files = 0;
        for name in names {
            files += copy_tree(&from.join(&name), &to.join(&name), rewrite)?;
        }
        return Ok(files);
    }
    if !meta.is_file() {
        return Ok(0);
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot make {}: {e}", parent.display()))?;
    }
    let write_error = |e: std::io::Error| format!("cannot write {}: {e}", to.display());
    match rewrite {
        Some((old, new)) if from.extension() == Some(OsStr::new("jsonl")) => {
            let text = fs::read(from).map_err(error)?;
            let text = String::from_utf8_lossy(&text);
            fs::write(to, rewrite_session_id(&text, old, new)).map_err(write_error)?;
        }
        _ => {
            fs::copy(from, to).map_err(write_error)?;
        }
    }
    Ok(1)
}

/// `oauthAccount.organizationUuid` from the `.claude.json` of `account`.
fn organization(account: &Account, home: &OsStr) -> Option<String> {
    let bytes = fs::read(usage::claude_json_path(account, home)).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value
        .pointer("/oauthAccount/organizationUuid")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// The warning when the two accounts are in different organizations
/// (ADR 0007). With a missing value, there is no warning.
pub fn organization_warning(
    source: &str,
    target: &str,
    source_org: Option<&str>,
    target_org: Option<&str>,
) -> Option<String> {
    match (source_org, target_org) {
        (Some(a), Some(b)) if a != b => Some(format!(
            "{source} and {target} are in different organizations. Claude cannot use the \
             earlier thinking of this session in {target}. The next responses can be slower \
             and use more tokens"
        )),
        _ => None,
    }
}

/// Copies session `thread` from `source` into `target`. `warn` gets the
/// warnings. Handoff only reads the source files.
pub fn handoff(
    home: &OsStr,
    source: &Account,
    target: &Account,
    thread: &Thread,
    now: SystemTime,
    warn: &mut dyn FnMut(String),
) -> Result<Done> {
    let id = thread.id.as_str();
    if source.name == target.name {
        return Err(format!(
            "session {id} is in account {} already; select a different target account",
            target.name
        ));
    }
    for account in [source, target] {
        if let Some(pid) = open_pid(&account.dir, id) {
            return Err(format!(
                "session {id} is open in account {} (pid {pid}). Close it, then try again",
                account.name
            ));
        }
    }
    let age = now.duration_since(thread.modified).unwrap_or_default();
    if age < RECENT {
        return Err(format!(
            "session {id} changed in the last {} seconds in account {}. It is probably \
             open. Close it, then try again",
            RECENT.as_secs(),
            source.name
        ));
    }
    let bytes = fs::read(&thread.path)
        .map_err(|e| format!("cannot read {}: {e}", thread.path.display()))?;
    let enc = thread
        .path
        .parent()
        .and_then(Path::file_name)
        .ok_or_else(|| format!("{} is not in projects/", thread.path.display()))?;
    let project = Path::new("projects").join(enc);
    let existing = fs::read(target.dir.join(&project).join(format!("{id}.jsonl"))).ok();
    let plan = plan(existing.as_deref(), &bytes);
    let new = match plan {
        Plan::Fork => new_session_id()?,
        Plan::New | Plan::Overwrite => id.to_string(),
    };

    if let Some(w) = organization_warning(
        &source.name,
        &target.name,
        organization(source, home).as_deref(),
        organization(target, home).as_deref(),
    ) {
        warn(w);
    }

    let text = String::from_utf8_lossy(&bytes);
    let to = target.dir.join(&project).join(format!("{new}.jsonl"));
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot make {}: {e}", parent.display()))?;
    }
    let written = if plan == Plan::Fork {
        fs::write(&to, rewrite_session_id(&text, id, &new))
    } else {
        fs::write(&to, &bytes)
    };
    written.map_err(|e| format!("cannot write {}: {e}", to.display()))?;
    let mut files = 1;

    let rewrite = (plan == Plan::Fork).then_some((id, new.as_str()));
    for (from, to) in session_dirs(&source.dir, id, &new) {
        files += copy_tree(&source.dir.join(from), &target.dir.join(to), rewrite)?;
    }
    for rel in plan_files(&source.dir, &slugs(&text)) {
        let to = target.dir.join(&rel);
        // A fork keeps a different plan file of the target copy.
        if plan == Plan::Fork
            && to.exists()
            && fs::read(&to).ok() != fs::read(source.dir.join(&rel)).ok()
        {
            warn(format!(
                "{} has a different {} already; ccx did not change it",
                target.name,
                rel.display()
            ));
            continue;
        }
        files += copy_tree(&source.dir.join(&rel), &to, None)?;
    }

    let cwd = summary(text.as_bytes()).cwd.map(PathBuf::from);
    Ok(Done {
        id: new,
        plan,
        files,
        cwd,
    })
}

/// Finds the source of session `id`: an exact id or a unique prefix of 8
/// or more characters, in an account other than `target`. When more than
/// one account has the session, the newest copy wins.
fn find_source(config: &Config, id: &str, target: Option<&Account>) -> Result<Thread> {
    if !is_session_id(id) {
        return Err(format!("`{id}` is not a session id"));
    }
    let matches = |t: &Thread| t.id == id || (id.len() >= 8 && t.id.starts_with(id));
    let all: Vec<Thread> = list(config, None).into_iter().filter(matches).collect();
    let ids: BTreeSet<&str> = all.iter().map(|t| t.id.as_str()).collect();
    if ids.len() > 1 {
        let ids: Vec<&str> = ids.into_iter().collect();
        return Err(format!(
            "`{id}` agrees with more than one session: {}",
            ids.join(", ")
        ));
    }
    let not_target = |t: &&Thread| target.is_none_or(|a| a.name != t.account);
    match all.iter().find(not_target) {
        Some(thread) => Ok(thread.clone()),
        None => match (all.first(), target) {
            (Some(t), Some(a)) => Err(format!(
                "session {} is only in account {}; there is nothing to copy",
                t.id, a.name
            )),
            _ => Err(format!("no session {id} in any account")),
        },
    }
}

fn account<'a>(config: &'a Config, text: &str) -> Result<&'a Account> {
    let name = text.strip_prefix('@').unwrap_or(text);
    config.account(name).ok_or_else(|| {
        format!(
            "unknown account `{name}`; known accounts: {}",
            config.names().join(", ")
        )
    })
}

/// The physical path of the current directory.
pub fn physical_cwd() -> Result<String> {
    let cwd = std::env::current_dir()
        .and_then(fs::canonicalize)
        .map_err(|e| format!("cannot get the current directory: {e}"))?;
    Ok(cwd.to_string_lossy().into_owned())
}

/// `ccx handoff [--all] [<id>] [@name] [--no-resume]`.
pub fn run(env: &Env, args: Args) -> Result<ExitCode> {
    let config = config::load(env)?;
    let home = config::home(env)?;
    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    let (mut id, mut target) = (args.id, args.target);
    if target.is_none() && id.as_deref().is_some_and(|i| i.starts_with('@')) {
        target = id.take();
    }
    let target = target.map(|t| account(&config, &t)).transpose()?;

    let thread = match id {
        Some(id) => find_source(&config, &id, target)?,
        None => {
            let cwd = physical_cwd()?;
            let enc = (!args.all).then(|| encode_cwd(&cwd));
            let threads = list(&config, enc.as_deref());
            if threads.is_empty() {
                let place = if args.all {
                    String::new()
                } else {
                    format!(" for {cwd}")
                };
                eprintln!("ccx: no threads{place} in any account");
                return Ok(ExitCode::SUCCESS);
            }
            let rows = rows(&threads, args.all, home);
            if !interactive {
                for row in rows {
                    println!("{row}");
                }
                return Ok(ExitCode::SUCCESS);
            }
            let Some(index) = dialoguer::Select::new()
                .with_prompt("Select a thread")
                .items(&rows)
                .default(0)
                .interact_opt()
                .map_err(|e| format!("cannot read the answer: {e}"))?
            else {
                return Ok(ExitCode::SUCCESS);
            };
            threads[index].clone()
        }
    };
    let target = match target {
        Some(target) => target,
        None if interactive => {
            let others: Vec<&Account> = config
                .accounts
                .iter()
                .filter(|a| a.name != thread.account)
                .collect();
            if others.is_empty() {
                return Err("there is no other account".into());
            }
            let names: Vec<&str> = others.iter().map(|a| a.name.as_str()).collect();
            let Some(index) = dialoguer::Select::new()
                .with_prompt("Select the target account")
                .items(&names)
                .default(0)
                .interact_opt()
                .map_err(|e| format!("cannot read the answer: {e}"))?
            else {
                return Ok(ExitCode::SUCCESS);
            };
            others[index]
        }
        None => {
            return Err(format!(
                "name the target account, for example `ccx handoff {} @name`",
                thread.id
            ));
        }
    };
    let source = config
        .account(&thread.account)
        .expect("each thread is in a configured account");

    let done = handoff(home, source, target, &thread, SystemTime::now(), &mut |w| {
        eprintln!("ccx: warning: {w}")
    })?;
    match done.plan {
        Plan::Fork => eprintln!(
            "ccx: {} has a different copy of session {}. ccx wrote a fork with the new session id {}",
            target.name, thread.id, done.id
        ),
        Plan::New | Plan::Overwrite => eprintln!(
            "ccx: copied session {} from {} to {} ({} files)",
            done.id, source.name, target.name, done.files
        ),
    }
    if args.no_resume {
        println!("{}", done.id);
        return Ok(ExitCode::SUCCESS);
    }
    resume(&config, env, target, &done)
}

/// Replaces ccx with `claude --resume <id>` in `target`, with the same env
/// rules as `ccx @name`.
fn resume(config: &Config, env: &Env, target: &Account, done: &Done) -> Result<ExitCode> {
    let depth = launch::check_depth(env)?;
    let self_exe = std::env::current_exe().ok();
    let Some(claude) = launch::find_claude(
        config.launch.claude_bin.as_deref(),
        env,
        self_exe.as_deref(),
    ) else {
        eprintln!("ccx: cannot find claude; set launch.claude_bin or CCX_CLAUDE_BIN");
        eprintln!(
            "ccx: to resume, run `ccx @{} --resume {}`",
            target.name, done.id
        );
        return Ok(ExitCode::from(EXIT_EXEC_FAILED));
    };
    let (child_env, _) = launch::build_env(env, target, depth);
    let mut cmd = Command::new(&claude);
    cmd.args(["--resume", done.id.as_str()])
        .env_clear()
        .envs(child_env.iter());
    // claude finds the session in the `projects/` directory of the cwd.
    if let Some(dir) = done.cwd.as_ref().filter(|d| d.is_dir()) {
        cmd.current_dir(dir);
    }
    let error = cmd.exec();
    eprintln!("ccx: cannot run {}: {error}", claude.display());
    Ok(ExitCode::from(EXIT_EXEC_FAILED))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The golden examples of ADR 0006.
    #[test]
    fn cwd_encoding_golden() {
        let long = format!("/Users/example/{}", "x".repeat(200));
        let long_want = format!("-Users-example-{}-6k8g1n", "x".repeat(185));
        let cases = [
            ("/Users/example/projects/ccx", "-Users-example-projects-ccx"),
            (
                "/Users/example/.agents/mcp-servers/inkwire",
                "-Users-example--agents-mcp-servers-inkwire",
            ),
            ("/private/tmp/a_b.c d+é~x@y", "-private-tmp-a-b-c-d---x-y"),
            ("/Users/example/emo😀j", "-Users-example-emo--j"),
            (long.as_str(), long_want.as_str()),
        ];
        for (cwd, want) in cases {
            assert_eq!(encode_cwd(cwd), want, "{cwd}");
        }
        assert_eq!(long_want.len(), 207);
    }

    #[test]
    fn cwd_encoding_edges() {
        // Exactly 200 characters: no hash.
        let path = format!("/{}", "a".repeat(199));
        assert_eq!(encode_cwd(&path).len(), 200);
        assert_eq!(base36(0), "0");
        assert_eq!(base36(35), "z");
        assert_eq!(base36(36), "10");
        assert_eq!(java_hash(""), 0);
        assert_eq!(java_hash("a"), 97);
        // A hash of i32::MIN has the absolute value 2^31.
        assert_eq!(base36(u64::from(i32::MIN.unsigned_abs())), "zik0zk");
    }

    #[test]
    fn plan_rule() {
        assert_eq!(plan(None, b"a\nb\n"), Plan::New);
        assert_eq!(plan(Some(b""), b"a\nb\n"), Plan::Overwrite);
        assert_eq!(plan(Some(b"a\n"), b"a\nb\n"), Plan::Overwrite);
        assert_eq!(plan(Some(b"a\nb\n"), b"a\nb\n"), Plan::Overwrite);
        assert_eq!(plan(Some(b"a\nc\n"), b"a\nb\n"), Plan::Fork);
        assert_eq!(plan(Some(b"a\nb\nc\n"), b"a\nb\n"), Plan::Fork);
    }

    #[test]
    fn slugs_are_distinct_and_in_order() {
        let text = r#"{"type":"user","slug":"staged-beacon"}
{"type":"assistant"}
not json "slug"
{"type":"user","slug":"staged-beacon"}
{"type":"user","slug":"other-plan_2"}
{"type":"user","slug":"../escape"}
{"type":"user","message":{"slug":"nested"}}
"#;
        assert_eq!(slugs(text), ["staged-beacon", "other-plan_2"]);
        assert!(slugs("{\"type\":\"user\"}\n").is_empty());
    }

    #[test]
    fn plan_files_for_slugs() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("plans")).unwrap();
        fs::write(dir.path().join("plans/staged-beacon.md"), "plan").unwrap();
        let slugs = vec!["staged-beacon".to_string(), "no-file".to_string()];
        assert_eq!(
            plan_files(dir.path(), &slugs),
            [PathBuf::from("plans/staged-beacon.md")]
        );
    }

    #[test]
    fn summary_gets_the_first_real_prompt_and_cwd() {
        let text = r#"{"type":"summary","summary":"x"}
{"type":"user","isMeta":true,"cwd":"/w","message":{"content":"meta"}}
{"type":"user","message":{"content":"<command-name>/clear</command-name>"}}
{"type":"user","message":{"content":[{"type":"tool_result","content":"r"}]}}
{"type":"user","message":{"content":[{"type":"text","text":"fix  the\nbug"}]}}
{"type":"user","message":{"content":"later"}}
"#;
        let s = summary(text.as_bytes());
        assert_eq!(s.prompt.as_deref(), Some("fix the bug"));
        assert_eq!(s.cwd.as_deref(), Some("/w"));
        assert_eq!(summary(&b""[..]), Summary::default());
    }

    #[test]
    fn shorten_text() {
        assert_eq!(shorten("abc", 3), "abc");
        assert_eq!(shorten("abcd", 3), "ab…");
    }

    #[test]
    fn session_file_names() {
        assert!(is_session_file("12345.json"));
        assert!(!is_session_file("12345.0123abcd.key"));
        assert!(!is_session_file("12345.json.key"));
        assert!(!is_session_file(".json"));
        assert!(!is_session_file("x.json"));
    }

    #[test]
    fn session_ids() {
        assert!(is_session_id("00000000-0000-4000-8000-000000000000"));
        assert!(is_session_id("abc12345"));
        assert!(!is_session_id("../x"));
        assert!(!is_session_id("memory"));
        assert!(!is_session_id(""));
    }

    #[test]
    fn new_session_ids_are_version_4() {
        let id = new_session_id().unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(id, new_session_id().unwrap());
    }

    #[test]
    fn rewrite_changes_only_the_field() {
        let line = r#"{"sessionId":"old-id","message":{"content":"{\"sessionId\":\"old-id\"}"}}"#;
        let want = r#"{"sessionId":"new-id","message":{"content":"{\"sessionId\":\"old-id\"}"}}"#;
        assert_eq!(rewrite_session_id(line, "old-id", "new-id"), want);
    }

    #[test]
    fn organization_warning_rule() {
        assert!(organization_warning("a", "b", Some("x"), Some("y")).is_some());
        assert!(organization_warning("a", "b", Some("x"), Some("x")).is_none());
        assert!(organization_warning("a", "b", None, Some("y")).is_none());
        assert!(organization_warning("a", "b", Some("x"), None).is_none());
    }

    #[test]
    fn process_check_uses_the_start_time() {
        let me = std::process::id();
        assert!(is_running(me, None));
        assert!(!is_running(me, Some("Thu Jan  1 00:00:00 1970")));
        if let Some(start) = process_start(me) {
            assert!(is_running(me, Some(&start)));
        }
        let mut child = Command::new("true").spawn().unwrap();
        let dead = child.id();
        child.wait().unwrap();
        assert!(!is_running(dead, None));
        assert!(!is_running(0, None));
    }
}

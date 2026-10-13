//! `ccx manage service install|uninstall|status` (#13): an optional job
//! that runs `ccx usage refresh --all --background` every 300 seconds.
//!
//! - macOS: a launchd agent in `~/Library/LaunchAgents/`, loaded with
//!   `launchctl`.
//! - Linux with systemd: a user service and a user timer in
//!   `~/.config/systemd/user/`, enabled with `systemctl --user`.
//! - Other systems: ccx prints a crontab line. ccx never edits the crontab.
//!
//! The job has the absolute paths of ccx and `claude` and a minimal `PATH`
//! (ADR 0007). A launchd job with a minimal `PATH` cannot find a bare
//! `claude`, so the job gives the path in `CCX_CLAUDE_BIN`. The path is
//! found at install time. `status` and `doctor` show an error when it no
//! longer exists.
//!
//! The job uses the background refresh of #11, so it obeys the refresh lock
//! and the `usage.refresh_min_gap_secs` rule.

use std::ffi::OsStr;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::adopt::write_atomic;
use crate::config::{self, Result};
use crate::env::Env;
use crate::launch;
use crate::paths::tilde;
use crate::pick;
use crate::refresh;
use crate::usage;

/// The fixed time between two runs of the job.
pub const INTERVAL_SECS: u32 = 300;
/// The launchd label and the name of the plist file.
pub const LABEL: &str = "io.github.thetechchild.ccx.refresh";
/// The name of the systemd units, with no suffix.
pub const UNIT: &str = "ccx-refresh";
/// The `PATH` of the launchd job in ADR 0007. The job adds the directory
/// of `claude` before it.
pub const MINIMAL_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
/// The arguments of the job after the ccx path.
pub const JOB_ARGS: [&str; 4] = ["usage", "refresh", "--all", "--background"];
/// The exit code of `status` when a path in the job does not exist.
pub const EXIT_BROKEN: u8 = 1;

/// The service manager of this system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Launchd,
    Systemd,
    Cron,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Launchd => "launchd",
            Kind::Systemd => "systemd",
            Kind::Cron => "cron",
        }
    }
}

/// The service manager: launchd on macOS, systemd when
/// `/run/systemd/system` exists, else cron. `CCX_SERVICE_KIND` (`launchd`,
/// `systemd` or `cron`) overrides the result, for tests.
pub fn detect(env: &Env) -> Result<Kind> {
    if let Some(value) = env.get("CCX_SERVICE_KIND") {
        return match value.to_str() {
            Some("launchd") => Ok(Kind::Launchd),
            Some("systemd") => Ok(Kind::Systemd),
            Some("cron") => Ok(Kind::Cron),
            _ => Err(format!(
                "CCX_SERVICE_KIND must be launchd, systemd or cron, not {}",
                value.to_string_lossy()
            )),
        };
    }
    Ok(if cfg!(target_os = "macos") {
        Kind::Launchd
    } else if Path::new("/run/systemd/system").is_dir() {
        Kind::Systemd
    } else {
        Kind::Cron
    })
}

/// The values that go into the job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// The absolute path of the ccx binary.
    pub ccx: String,
    /// The absolute path of `claude`, found at install time.
    pub claude: String,
    /// Other variables for the job, for example `CCX_CONFIG`.
    pub extra_env: Vec<(&'static str, String)>,
}

/// Variables that change where ccx finds its config. The job keeps them
/// when they are set at install time.
const KEEP_VARS: [&str; 2] = ["CCX_CONFIG", "XDG_CONFIG_HOME"];

impl Job {
    /// Makes the job from the paths. It refuses a path that is not
    /// absolute, or that has a character that a service file or a crontab
    /// line cannot hold safely.
    pub fn new(ccx: &Path, claude: &Path, env: &Env) -> Result<Job> {
        let mut extra_env = Vec::new();
        for key in KEEP_VARS {
            if let Some(value) = env.get(key) {
                extra_env.push((key, safe_text(Path::new(value), key)?));
            }
        }
        Ok(Job {
            ccx: safe_text(ccx, "the ccx path")?,
            claude: safe_text(claude, "the claude path")?,
            extra_env,
        })
    }

    /// The directory of `claude`, then the minimal `PATH` of ADR 0007.
    pub fn path_var(&self) -> String {
        match Path::new(&self.claude).parent().and_then(Path::to_str) {
            Some(dir) if !dir.is_empty() && !MINIMAL_PATH.split(':').any(|d| d == dir) => {
                format!("{dir}:{MINIMAL_PATH}")
            }
            _ => MINIMAL_PATH.to_string(),
        }
    }

    /// The environment of the job, in a fixed order.
    pub fn env_vars(&self) -> Vec<(&str, String)> {
        let mut vars = vec![
            ("PATH", self.path_var()),
            ("CCX_CLAUDE_BIN", self.claude.clone()),
        ];
        vars.extend(self.extra_env.iter().map(|(k, v)| (*k, v.clone())));
        vars
    }
}

/// The text of `path` when it is absolute and has no character from
/// `"'\%:`, no control character and no character that is not UTF-8.
fn safe_text(path: &Path, what: &str) -> Result<String> {
    let text = path
        .to_str()
        .ok_or_else(|| format!("{what} is not UTF-8: {}", path.display()))?;
    if !path.is_absolute() {
        return Err(format!("{what} is not absolute: {text}"));
    }
    if let Some(c) = text
        .chars()
        .find(|c| c.is_control() || "\"'\\%:".contains(*c))
    {
        return Err(format!(
            "{what} has the character {c:?}, which a service file cannot hold: {text}"
        ));
    }
    Ok(text.to_string())
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn xml_unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The launchd plist: `RunAtLoad` and a `StartInterval` of 300 seconds.
pub fn plist(job: &Job) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n",
    );
    out.push_str(&format!("\t<key>Label</key>\n\t<string>{LABEL}</string>\n"));
    out.push_str("\t<key>ProgramArguments</key>\n\t<array>\n");
    for arg in std::iter::once(job.ccx.as_str()).chain(JOB_ARGS) {
        out.push_str(&format!("\t\t<string>{}</string>\n", xml_escape(arg)));
    }
    out.push_str("\t</array>\n\t<key>EnvironmentVariables</key>\n\t<dict>\n");
    for (key, value) in job.env_vars() {
        out.push_str(&format!(
            "\t\t<key>{key}</key>\n\t\t<string>{}</string>\n",
            xml_escape(&value)
        ));
    }
    out.push_str("\t</dict>\n");
    out.push_str("\t<key>RunAtLoad</key>\n\t<true/>\n");
    out.push_str(&format!(
        "\t<key>StartInterval</key>\n\t<integer>{INTERVAL_SECS}</integer>\n"
    ));
    out.push_str("\t<key>ProcessType</key>\n\t<string>Background</string>\n");
    out.push_str("</dict>\n</plist>\n");
    out
}

/// The systemd user service. The timer starts it.
pub fn systemd_service(job: &Job) -> String {
    let mut out =
        String::from("[Unit]\nDescription=ccx usage refresh\n\n[Service]\nType=oneshot\n");
    for (key, value) in job.env_vars() {
        out.push_str(&format!("Environment=\"{key}={value}\"\n"));
    }
    out.push_str(&format!(
        "ExecStart=\"{}\" {}\n",
        job.ccx,
        JOB_ARGS.join(" ")
    ));
    out
}

/// The systemd user timer: it starts the service 1 minute after the timer starts,
/// then 300 seconds after each run.
pub fn systemd_timer() -> String {
    format!(
        "[Unit]\nDescription=ccx usage refresh every {INTERVAL_SECS} s\n\n\
         [Timer]\nOnActiveSec=1min\nOnUnitActiveSec={INTERVAL_SECS}\nUnit={UNIT}.service\n\n\
         [Install]\nWantedBy=timers.target\n"
    )
}

/// The crontab line for a system with no systemd: each 5 minutes.
pub fn crontab_line(job: &Job) -> String {
    let vars: Vec<String> = job
        .env_vars()
        .into_iter()
        .map(|(key, value)| format!("{key}='{value}'"))
        .collect();
    format!(
        "*/5 * * * * {} '{}' {} >/dev/null 2>&1",
        vars.join(" "),
        job.ccx,
        JOB_ARGS.join(" ")
    )
}

/// The paths in an installed job file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Recorded {
    pub ccx: Option<PathBuf>,
    pub claude: Option<PathBuf>,
}

/// Reads the ccx path and the `claude` path from a plist that `plist` wrote.
pub fn read_plist(text: &str) -> Recorded {
    let string_after = |marker: &str| -> Option<PathBuf> {
        let rest = &text[text.find(marker)? + marker.len()..];
        let start = rest.find("<string>")? + "<string>".len();
        let end = start + rest[start..].find("</string>")?;
        Some(PathBuf::from(xml_unescape(&rest[start..end])))
    };
    Recorded {
        ccx: string_after("<key>ProgramArguments</key>"),
        claude: string_after("<key>CCX_CLAUDE_BIN</key>"),
    }
}

/// Reads the ccx path and the `claude` path from a unit that
/// `systemd_service` wrote.
pub fn read_unit(text: &str) -> Recorded {
    let mut out = Recorded::default();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("ExecStart=\"") {
            out.ccx = rest.split('"').next().map(PathBuf::from);
        } else if let Some(rest) = line.strip_prefix("Environment=\"CCX_CLAUDE_BIN=") {
            out.claude = rest.strip_suffix('"').map(PathBuf::from);
        }
    }
    out
}

/// The files that `install` writes for `kind`. Cron has none.
pub fn files(kind: Kind, env: &Env) -> Result<Vec<PathBuf>> {
    let home = Path::new(config::home(env)?);
    Ok(match kind {
        Kind::Launchd => vec![
            home.join("Library/LaunchAgents")
                .join(format!("{LABEL}.plist")),
        ],
        Kind::Systemd => {
            let dir = match env.get("XDG_CONFIG_HOME").map(Path::new) {
                Some(xdg) if xdg.is_absolute() => xdg.to_path_buf(),
                _ => home.join(".config"),
            }
            .join("systemd/user");
            vec![
                dir.join(format!("{UNIT}.service")),
                dir.join(format!("{UNIT}.timer")),
            ]
        }
        Kind::Cron => Vec::new(),
    })
}

/// The paths that the installed job file records, or `None` when no job
/// file exists.
pub fn installed(kind: Kind, env: &Env) -> Result<Option<(PathBuf, Recorded)>> {
    let Some(path) = files(kind, env)?.into_iter().next() else {
        return Ok(None);
    };
    match fs::read_to_string(&path) {
        Ok(text) => {
            let recorded = match kind {
                Kind::Launchd => read_plist(&text),
                _ => read_unit(&text),
            };
            Ok(Some((path, recorded)))
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// The problems with the paths of an installed job: a path that is
/// missing from the file or that is not an executable file.
pub fn problems(recorded: &Recorded) -> Vec<String> {
    let mut out = Vec::new();
    for (what, path) in [("ccx", &recorded.ccx), ("claude", &recorded.claude)] {
        match path {
            None => out.push(format!(
                "the job has no {what} path; run `ccx manage service install` again"
            )),
            Some(path) if !launch::is_executable(path) => out.push(format!(
                "the job uses {what} at {}, which does not exist; run `ccx manage service install` again",
                path.display()
            )),
            Some(_) => {}
        }
    }
    out
}

/// Runs a service tool, for example `launchctl`, with the environment of
/// ccx. The tool is found on `PATH`.
fn tool(env: &Env, program: &str, args: &[&str]) -> Result<Output> {
    Command::new(program)
        .args(args)
        .env_clear()
        .envs(env.iter())
        .output()
        .map_err(|e| format!("cannot run {program}: {e}"))
}

/// Runs a service tool and fails when its exit code is not 0.
fn tool_ok(env: &Env, program: &str, args: &[&str]) -> Result<()> {
    let out = tool(env, program, args)?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(format!(
        "`{program} {}` failed ({}): {}",
        args.join(" "),
        out.status,
        stderr.trim()
    ))
}

fn launchd_target() -> String {
    // SAFETY: `getuid` has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    format!("gui/{uid}")
}

/// True when the job is loaded (launchd) or the timer is active (systemd).
fn loaded(kind: Kind, env: &Env) -> Result<bool> {
    match kind {
        Kind::Launchd => {
            let service = format!("{}/{LABEL}", launchd_target());
            Ok(tool(env, "launchctl", &["print", &service])?
                .status
                .success())
        }
        Kind::Systemd => {
            let timer = format!("{UNIT}.timer");
            let out = tool(env, "systemctl", &["--user", "is-active", &timer])?;
            Ok(String::from_utf8_lossy(&out.stdout).trim() == "active")
        }
        Kind::Cron => Ok(false),
    }
}

/// The job for this ccx binary and the `claude` that ccx uses now.
fn current_job(env: &Env) -> Result<Job> {
    let config = config::load(env)?;
    let self_exe = std::env::current_exe().map_err(|e| format!("cannot find ccx: {e}"))?;
    let claude = launch::find_claude(config.launch.claude_bin.as_deref(), env, Some(&self_exe))
        .ok_or("cannot find claude; set launch.claude_bin or CCX_CLAUDE_BIN")?;
    let claude = std::path::absolute(&claude).map_err(|e| format!("{}: {e}", claude.display()))?;
    Job::new(&self_exe, &claude, env)
}

/// The text of each file for `kind`, in the order of `files`.
fn texts(kind: Kind, job: &Job) -> Vec<String> {
    match kind {
        Kind::Launchd => vec![plist(job)],
        Kind::Systemd => vec![systemd_service(job), systemd_timer()],
        Kind::Cron => Vec::new(),
    }
}

/// True when each file at `paths` has the text at the same index.
fn same_files(paths: &[PathBuf], texts: &[String]) -> bool {
    paths
        .iter()
        .zip(texts)
        .all(|(path, text)| fs::read_to_string(path).is_ok_and(|have| &have == text))
}

/// True when the job for this ccx and this `claude` is installed and
/// loaded, so `install` has nothing to change. Always false for cron.
pub fn is_current(env: &Env) -> Result<bool> {
    let kind = detect(env)?;
    if kind == Kind::Cron {
        return Ok(false);
    }
    let job = current_job(env)?;
    Ok(same_files(&files(kind, env)?, &texts(kind, &job)) && loaded(kind, env)?)
}

/// `ccx manage service install`.
pub fn install(env: &Env) -> Result<()> {
    let kind = detect(env)?;
    let job = current_job(env)?;
    let home = config::home(env)?;
    if kind == Kind::Cron {
        println!("ccx: this system has no launchd and no systemd. ccx does not edit the crontab.");
        println!("Add this line with `crontab -e`:");
        println!();
        println!("{}", crontab_line(&job));
        return Ok(());
    }
    let paths = files(kind, env)?;
    let texts = texts(kind, &job);
    let same = same_files(&paths, &texts);
    let was_loaded = loaded(kind, env)?;
    if same && was_loaded {
        println!("ccx: the service is installed and loaded; nothing changed");
    } else {
        if kind == Kind::Launchd && was_loaded {
            let service = format!("{}/{LABEL}", launchd_target());
            tool_ok(env, "launchctl", &["bootout", &service])?;
        }
        for (path, text) in paths.iter().zip(&texts) {
            write_atomic(path, text, 0o644)?;
        }
        match kind {
            Kind::Launchd => {
                let plist = paths[0].to_string_lossy();
                tool_ok(env, "launchctl", &["bootstrap", &launchd_target(), &plist])?;
            }
            _ => {
                let timer = format!("{UNIT}.timer");
                tool_ok(env, "systemctl", &["--user", "daemon-reload"])?;
                tool_ok(env, "systemctl", &["--user", "enable", "--now", &timer])?;
            }
        }
        for path in &paths {
            println!("ccx: wrote {}", tilde(path, home));
        }
        println!(
            "ccx: the service runs `ccx usage refresh --all --background` every {INTERVAL_SECS} s"
        );
        println!("ccx: claude: {}", job.claude);
    }
    if kind == Kind::Systemd && !linger(env) {
        let user = env
            .get("USER")
            .map_or_else(String::new, |u| format!(" {}", u.to_string_lossy()));
        println!(
            "ccx: linger is off, so the timer runs only while you are logged in. \
             To run it at all times, run `loginctl enable-linger{user}`."
        );
    }
    Ok(())
}

/// True when systemd keeps the user manager after logout. An unknown
/// value counts as off.
fn linger(env: &Env) -> bool {
    // SAFETY: `getuid` has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() }.to_string();
    tool(
        env,
        "loginctl",
        &["show-user", &uid, "--property=Linger", "--value"],
    )
    .is_ok_and(|out| out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "yes")
}

/// `ccx manage service uninstall`.
pub fn uninstall(env: &Env) -> Result<()> {
    let kind = detect(env)?;
    let home = config::home(env)?;
    if kind == Kind::Cron {
        println!(
            "ccx: no service exists on this system. ccx does not edit the crontab. \
             To stop a cron job, remove its `usage refresh --all --background` line with `crontab -e`."
        );
        return Ok(());
    }
    let paths = files(kind, env)?;
    let existing: Vec<&PathBuf> = paths.iter().filter(|p| p.exists()).collect();
    let was_loaded = loaded(kind, env)?;
    if existing.is_empty() && !was_loaded {
        println!("ccx: no service is installed");
        return Ok(());
    }
    match kind {
        Kind::Launchd if was_loaded => {
            let service = format!("{}/{LABEL}", launchd_target());
            tool_ok(env, "launchctl", &["bootout", &service])?;
        }
        Kind::Systemd => {
            let timer = format!("{UNIT}.timer");
            tool_ok(env, "systemctl", &["--user", "disable", "--now", &timer])?;
        }
        _ => {}
    }
    for path in existing {
        fs::remove_file(path).map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
        println!("ccx: removed {}", tilde(path, home));
    }
    if kind == Kind::Systemd {
        tool_ok(env, "systemctl", &["--user", "daemon-reload"])?;
    }
    println!("ccx: the service is stopped and removed");
    Ok(())
}

/// `ccx manage service status`. Returns the exit code: 0, or 1 when a path
/// in the installed job does not exist.
pub fn status(env: &Env) -> Result<u8> {
    let kind = detect(env)?;
    let home = config::home(env)?;
    let mut code = 0;
    let mut out = format!("service: {}\n", kind.label());
    if kind == Kind::Cron {
        out.push_str(
            "installed: unknown (ccx does not read the crontab; \
             `ccx manage service install` prints the line)\n",
        );
    } else {
        let paths = files(kind, env)?;
        let shown: Vec<String> = paths.iter().map(|p| tilde(p, home)).collect();
        match installed(kind, env)? {
            None => out.push_str(&format!("installed: no ({})\n", shown.join(", "))),
            Some((_, recorded)) => {
                out.push_str(&format!("installed: yes ({})\n", shown.join(", ")));
                if let Some(claude) = &recorded.claude {
                    out.push_str(&format!("claude: {}\n", tilde(claude, home)));
                }
                for problem in problems(&recorded) {
                    out.push_str(&format!("error: {problem}\n"));
                    code = EXIT_BROKEN;
                }
            }
        }
        let state = if loaded(kind, env)? { "yes" } else { "no" };
        out.push_str(&format!("loaded: {state}\n"));
    }
    let now = usage::now_ms();
    out.push_str(&format!(
        "last refresh: {}\n",
        last_refresh_text(refresh::last_start(home), now)
    ));
    out.push_str(&cache_ages(env, home, now));
    print!("{out}");
    Ok(code)
}

/// The start time of the last fetch, for example `14:05 (3m ago)`.
fn last_refresh_text(at_ms: Option<u64>, now_ms: u64) -> String {
    match at_ms {
        None => "never".into(),
        Some(at) => format!(
            "{} ({} ago)",
            pick::time_text(at as i64, now_ms, pick::local_clock),
            usage::short_duration(now_ms.saturating_sub(at) as i64)
        ),
    }
}

/// The age of the usage cache of each account.
fn cache_ages(env: &Env, home: &OsStr, now_ms: u64) -> String {
    let config = match config::load(env) {
        Ok(config) => config,
        Err(e) => return format!("usage cache: {e}\n"),
    };
    let width = config
        .accounts
        .iter()
        .map(|a| a.name.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = String::from("usage cache:\n");
    for account in &config.accounts {
        let age = usage::read_cache(&usage::cache_path(home, &account.name)).map_or_else(
            || "none".to_string(),
            |s| {
                format!(
                    "{} old",
                    usage::short_duration(now_ms.saturating_sub(s.fetched_at_ms) as i64)
                )
            },
        );
        out.push_str(&format!("  {:<width$}  {age}\n", account.name));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> Job {
        Job::new(
            Path::new("/home/test/.local/bin/ccx"),
            Path::new("/opt/claude/bin/claude"),
            &Env::default(),
        )
        .unwrap()
    }

    /// Compares `have` with the golden file `testdata/service/<name>`.
    /// `CCX_UPDATE_GOLDEN=1` writes the file.
    fn golden(name: &str, have: &str) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/service")
            .join(name);
        if std::env::var_os("CCX_UPDATE_GOLDEN").is_some() {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, have).unwrap();
            return;
        }
        let want = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(have, want, "{} differs", path.display());
    }

    #[test]
    fn golden_plist() {
        let text = plist(&job());
        golden("ccx-refresh.plist", &text);
        assert!(text.contains("<key>RunAtLoad</key>\n\t<true/>"));
        assert!(text.contains("<integer>300</integer>"));
    }

    #[test]
    fn golden_systemd_units() {
        golden("ccx-refresh.service", &systemd_service(&job()));
        let timer = systemd_timer();
        golden("ccx-refresh.timer", &timer);
        assert!(timer.contains("OnUnitActiveSec=300\n"));
    }

    #[test]
    fn golden_crontab_line() {
        golden("crontab.txt", &(crontab_line(&job()) + "\n"));
    }

    #[test]
    fn path_has_the_claude_dir_then_the_adr_path() {
        assert_eq!(
            job().path_var(),
            "/opt/claude/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        );
        let in_minimal = Job::new(
            Path::new("/x/ccx"),
            Path::new("/usr/bin/claude"),
            &Env::default(),
        )
        .unwrap();
        assert_eq!(in_minimal.path_var(), MINIMAL_PATH);
    }

    #[test]
    fn the_job_file_records_the_paths() {
        let want = Recorded {
            ccx: Some("/home/test/.local/bin/ccx".into()),
            claude: Some("/opt/claude/bin/claude".into()),
        };
        assert_eq!(read_plist(&plist(&job())), want);
        assert_eq!(read_unit(&systemd_service(&job())), want);
        assert_eq!(read_plist("<plist/>"), Recorded::default());
        assert_eq!(read_unit(""), Recorded::default());
    }

    #[test]
    fn xml_characters_are_escaped_and_read_back() {
        let job = Job::new(
            Path::new("/a & b/<c>/ccx"),
            Path::new("/opt/My Apps/claude"),
            &Env::default(),
        )
        .unwrap();
        let text = plist(&job);
        assert!(text.contains("<string>/a &amp; b/&lt;c&gt;/ccx</string>"));
        let recorded = read_plist(&text);
        assert_eq!(recorded.ccx, Some("/a & b/<c>/ccx".into()));
        assert_eq!(recorded.claude, Some("/opt/My Apps/claude".into()));
        assert_eq!(read_unit(&systemd_service(&job)).claude, recorded.claude);
    }

    #[test]
    fn unsafe_paths_are_refused() {
        let env = Env::default();
        for bad in [
            "relative/claude",
            "/a\"b/claude",
            "/a'b/claude",
            "/a\\b/claude",
            "/a%b/claude",
            "/a:b/claude",
            "/a\nb/claude",
        ] {
            assert!(
                Job::new(Path::new("/x/ccx"), Path::new(bad), &env).is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn config_variables_go_into_the_job() {
        let env: Env = [
            ("CCX_CONFIG", "/home/test/dots/ccx.toml"),
            ("XDG_CONFIG_HOME", "/home/test/xdg"),
        ]
        .into_iter()
        .collect();
        let job = Job::new(Path::new("/x/ccx"), Path::new("/y/claude"), &env).unwrap();
        let keys: Vec<&str> = job.env_vars().into_iter().map(|(k, _)| k).collect();
        assert_eq!(
            keys,
            ["PATH", "CCX_CLAUDE_BIN", "CCX_CONFIG", "XDG_CONFIG_HOME"]
        );
        assert!(
            systemd_service(&job).contains("Environment=\"CCX_CONFIG=/home/test/dots/ccx.toml\"\n")
        );
    }

    #[test]
    fn missing_paths_are_problems() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("ccx");
        fs::write(&exe, "#!/bin/sh\n").unwrap();
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        let good = Recorded {
            ccx: Some(exe.clone()),
            claude: Some(exe.clone()),
        };
        assert!(problems(&good).is_empty());
        let moved = Recorded {
            ccx: Some(exe),
            claude: Some(dir.path().join("gone/claude")),
        };
        let found = problems(&moved);
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("does not exist; run `ccx manage service install` again"));
        assert_eq!(problems(&Recorded::default()).len(), 2);
    }

    #[test]
    fn ccx_service_kind_overrides_the_detection() {
        let env = |v: &str| -> Env { [("CCX_SERVICE_KIND", v)].into_iter().collect() };
        assert_eq!(detect(&env("launchd")).unwrap(), Kind::Launchd);
        assert_eq!(detect(&env("systemd")).unwrap(), Kind::Systemd);
        assert_eq!(detect(&env("cron")).unwrap(), Kind::Cron);
        assert!(detect(&env("upstart")).is_err());
        if cfg!(target_os = "macos") {
            assert_eq!(detect(&Env::default()).unwrap(), Kind::Launchd);
        }
    }

    #[test]
    fn last_refresh_text_shows_never_or_the_age() {
        assert_eq!(last_refresh_text(None, 1_000), "never");
        let now = 10 * 60_000;
        let text = last_refresh_text(Some(now - 3 * 60_000), now);
        assert!(text.ends_with(" (3m ago)"), "{text}");
    }
}

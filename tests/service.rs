//! End-to-end tests for `ccx manage service install|uninstall|status` (#13).
//! Each test uses a temp HOME and the fakes in `testdata/fakebin`:
//! `claude`, `launchctl`, `systemctl` and `loginctl`. `CCX_SERVICE_KIND`
//! selects the service manager, so all kinds run on all platforms. No test
//! loads a real job or touches a real account.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::json;

const CCX: &str = env!("CARGO_BIN_EXE_ccx");
const MINIMAL_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
const PLIST: &str = "Library/LaunchAgents/io.github.thetechchild.ccx.refresh.plist";
const UNIT_DIR: &str = ".config/systemd/user";

const CONFIG: &str = r#"
[accounts.main]
dir = "~/.claude"

[accounts.angel2]
dir = "~/.claude-angel2"
"#;

struct Fixture {
    home: tempfile::TempDir,
    /// Not in HOME, so that a file listing of HOME does not see it.
    fake: tempfile::TempDir,
    kind: &'static str,
}

impl Fixture {
    fn new(kind: &'static str) -> Fixture {
        let home = tempfile::tempdir().expect("temp HOME");
        let dir = home.path().join(".config/ccx");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.toml"), CONFIG).unwrap();
        fs::create_dir(home.path().join(".claude")).unwrap();
        fs::create_dir(home.path().join(".claude-angel2")).unwrap();
        Fixture {
            home,
            fake: tempfile::tempdir().expect("fake state"),
            kind,
        }
    }

    fn home(&self) -> PathBuf {
        fs::canonicalize(self.home.path()).unwrap()
    }

    fn fakebin() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fakebin")
    }

    fn ccx(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(CCX);
        cmd.args(args)
            .current_dir(self.home())
            .env_clear()
            .env("HOME", self.home())
            .env("USER", "tester")
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", Self::fakebin().display()),
            )
            .env("CCX_SERVICE_KIND", self.kind)
            .env("FAKE_SERVICE_STATE", self.fake.path())
            .env("FAKE_SERVICE_LOG", self.fake.path().join("calls.log"))
            .env("FAKE_CLAUDE_LOG", self.fake.path().join("claude.log"));
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.ccx(args).output().expect("run ccx")
    }

    /// Runs ccx and expects exit code 0. Returns stdout.
    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{args:?}: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// The calls to the fake service tools, then clears the log.
    fn take_calls(&self) -> Vec<String> {
        let path = self.fake.path().join("calls.log");
        let text = fs::read_to_string(&path).unwrap_or_default();
        let _ = fs::remove_file(&path);
        text.lines().map(String::from).collect()
    }

    fn usage_calls(&self) -> usize {
        fs::read_to_string(self.fake.path().join("claude.log"))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains("\"/usage\""))
            .count()
    }

    /// Each file in HOME with its content.
    fn snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, out);
                } else {
                    out.insert(path.clone(), fs::read(&path).unwrap());
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(&self.home(), &mut out);
        out
    }
}

fn uid() -> String {
    let out = Command::new("id").arg("-u").output().unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn claude_path() -> String {
    Fixture::fakebin().join("claude").display().to_string()
}

fn ccx_path() -> String {
    fs::canonicalize(CCX).unwrap().display().to_string()
}

/// The strings in the `ProgramArguments` array of the plist.
fn program_arguments(plist: &str) -> Vec<String> {
    let start = plist.find("<array>").unwrap();
    let end = plist.find("</array>").unwrap();
    plist[start..end]
        .lines()
        .filter_map(|l| {
            l.trim()
                .strip_prefix("<string>")?
                .strip_suffix("</string>")
                .map(String::from)
        })
        .collect()
}

#[test]
fn launchd_install_writes_and_loads_the_plist_and_is_idempotent() {
    let fx = Fixture::new("launchd");
    let out = fx.ok(&["manage", "service", "install"]);
    assert!(out.contains(&format!("wrote ~/{PLIST}")), "{out}");

    let plist = fs::read_to_string(fx.home().join(PLIST)).unwrap();
    assert!(plist.contains("<key>RunAtLoad</key>\n\t<true/>"), "{plist}");
    assert!(plist.contains("<key>StartInterval</key>\n\t<integer>60</integer>"));
    let args = program_arguments(&plist);
    assert_eq!(
        args[1..],
        ["usage", "refresh", "--all", "--background", "--scheduled"]
    );
    assert!(Path::new(&args[0]).is_absolute());
    assert_eq!(
        fs::canonicalize(&args[0]).unwrap().display().to_string(),
        ccx_path()
    );
    let fakebin = Fixture::fakebin().display().to_string();
    assert!(plist.contains(&format!(
        "<key>PATH</key>\n\t\t<string>{fakebin}:{MINIMAL_PATH}</string>"
    )));
    assert!(plist.contains(&format!(
        "<key>CCX_CLAUDE_BIN</key>\n\t\t<string>{}</string>",
        claude_path()
    )));
    let target = format!("gui/{}", uid());
    let label = format!("{target}/io.github.thetechchild.ccx.refresh");
    let plist_path = fx.home().join(PLIST).display().to_string();
    assert_eq!(
        fx.take_calls(),
        [
            format!("launchctl print {label}"),
            format!("launchctl bootstrap {target} {plist_path}"),
        ]
    );

    // A second run changes nothing.
    let before = fx.snapshot();
    let out = fx.ok(&["manage", "service", "install"]);
    assert!(out.contains("nothing changed"), "{out}");
    assert_eq!(fx.snapshot(), before);
    assert_eq!(fx.take_calls(), [format!("launchctl print {label}")]);
}

#[test]
fn launchd_install_reloads_a_job_with_a_different_claude_path() {
    let fx = Fixture::new("launchd");
    fx.ok(&["manage", "service", "install"]);
    fx.take_calls();
    let other = fx.fake.path().join("bin/claude");
    fs::create_dir_all(other.parent().unwrap()).unwrap();
    fs::copy(Fixture::fakebin().join("claude"), &other).unwrap();
    let out = fx
        .ccx(&["manage", "service", "install"])
        .env("CCX_CLAUDE_BIN", &other)
        .output()
        .unwrap();
    assert!(out.status.success());
    let calls = fx.take_calls();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert!(calls[1].starts_with("launchctl bootout "));
    assert!(calls[2].starts_with("launchctl bootstrap "));
    let plist = fs::read_to_string(fx.home().join(PLIST)).unwrap();
    assert!(plist.contains(&format!("<string>{}</string>", other.display())));
}

#[test]
fn launchd_uninstall_stops_the_job_and_removes_the_plist() {
    let fx = Fixture::new("launchd");
    let before = fx.snapshot();
    fx.ok(&["manage", "service", "install"]);
    fx.take_calls();
    let out = fx.ok(&["manage", "service", "uninstall"]);
    assert!(out.contains(&format!("removed ~/{PLIST}")), "{out}");
    let calls = fx.take_calls();
    assert!(calls[1].starts_with("launchctl bootout gui/"), "{calls:?}");
    assert!(!fx.home().join(PLIST).exists());
    assert!(!fx.fake.path().join("loaded").exists());
    // Only empty directories stay.
    assert_eq!(fx.snapshot(), before);

    let out = fx.ok(&["manage", "service", "uninstall"]);
    assert!(out.contains("no service is installed"), "{out}");
}

#[test]
fn systemd_install_writes_the_units_enables_the_timer_and_shows_the_linger_hint() {
    let fx = Fixture::new("systemd");
    let out = fx.ok(&["manage", "service", "install"]);
    let service = fs::read_to_string(fx.home().join(UNIT_DIR).join("ccx-refresh.service")).unwrap();
    let timer = fs::read_to_string(fx.home().join(UNIT_DIR).join("ccx-refresh.timer")).unwrap();
    let exec = service
        .lines()
        .find_map(|l| l.strip_prefix("ExecStart="))
        .unwrap();
    let (exe, args) = exec.strip_prefix('"').unwrap().split_once("\" ").unwrap();
    assert_eq!(
        fs::canonicalize(exe).unwrap().display().to_string(),
        ccx_path()
    );
    assert_eq!(args, "usage refresh --all --background --scheduled");
    assert!(service.contains(&format!(
        "Environment=\"PATH={}:{MINIMAL_PATH}\"\n",
        Fixture::fakebin().display()
    )));
    assert!(service.contains(&format!(
        "Environment=\"CCX_CLAUDE_BIN={}\"\n",
        claude_path()
    )));
    assert!(timer.contains("OnUnitActiveSec=60\n"), "{timer}");
    assert!(timer.contains("WantedBy=timers.target\n"));

    let calls = fx.take_calls();
    assert!(
        calls.contains(&"systemctl --user daemon-reload".to_string()),
        "{calls:?}"
    );
    assert!(calls.contains(&"systemctl --user enable --now ccx-refresh.timer".to_string()));
    assert!(out.contains("loginctl enable-linger tester"), "{out}");

    // A second run with linger on changes nothing and shows no hint.
    let before = fx.snapshot();
    let out = fx
        .ccx(&["manage", "service", "install"])
        .env("FAKE_LINGER", "yes")
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(out.status.success());
    assert!(stdout.contains("nothing changed"), "{stdout}");
    assert!(!stdout.contains("enable-linger"), "{stdout}");
    assert_eq!(fx.snapshot(), before);
    assert!(!fx.take_calls().iter().any(|c| c.contains("enable --now")));
}

#[test]
fn systemd_uninstall_disables_the_timer_and_removes_the_units() {
    let fx = Fixture::new("systemd");
    fx.ok(&["manage", "service", "install"]);
    fx.take_calls();
    fx.ok(&["manage", "service", "uninstall"]);
    let calls = fx.take_calls();
    assert!(
        calls.contains(&"systemctl --user disable --now ccx-refresh.timer".to_string()),
        "{calls:?}"
    );
    assert_eq!(calls.last().unwrap(), "systemctl --user daemon-reload");
    assert!(
        !fx.home()
            .join(UNIT_DIR)
            .join("ccx-refresh.service")
            .exists()
    );
    assert!(!fx.home().join(UNIT_DIR).join("ccx-refresh.timer").exists());
    assert!(!fx.fake.path().join("active").exists());
}

#[test]
fn cron_install_prints_a_crontab_line_and_changes_no_files() {
    let fx = Fixture::new("cron");
    let before = fx.snapshot();
    let out = fx.ok(&["manage", "service", "install"]);
    let line = out
        .lines()
        .find(|l| l.starts_with("* * * * * "))
        .unwrap_or_else(|| panic!("{out}"));
    assert!(line.contains(&format!(
        "PATH='{}:{MINIMAL_PATH}'",
        Fixture::fakebin().display()
    )));
    assert!(line.contains(&format!("CCX_CLAUDE_BIN='{}'", claude_path())));
    assert!(line.ends_with(" usage refresh --all --background --scheduled >/dev/null 2>&1"));
    assert_eq!(fx.snapshot(), before);
    assert!(fx.take_calls().is_empty());

    let out = fx.ok(&["manage", "service", "uninstall"]);
    assert!(out.contains("crontab -e"), "{out}");
    assert_eq!(fx.snapshot(), before);
}

#[test]
fn status_works_when_no_service_is_installed() {
    for kind in ["launchd", "systemd", "cron"] {
        let fx = Fixture::new(kind);
        let out = fx.ok(&["manage", "service", "status"]);
        assert!(out.starts_with(&format!("service: {kind}\n")), "{out}");
        if kind != "cron" {
            assert!(out.contains("installed: no"), "{out}");
            assert!(out.contains("loaded: no\n"), "{out}");
        }
        assert!(out.contains("last refresh: never\n"), "{out}");
        assert!(
            out.contains(
                "usage cache:\n  angel2  none, next fetch: now\n  main    none, next fetch: now\n"
            ),
            "{out}"
        );
    }
}

#[test]
fn status_shows_the_installed_job_the_last_refresh_and_the_cache_age() {
    let fx = Fixture::new("launchd");
    fx.ok(&["manage", "service", "install"]);
    let state = fx.home().join(".local/state/ccx");
    fs::create_dir_all(state.join("usage")).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    fs::write(
        state.join("refresh.json"),
        json!({"started_at_ms": now - 3 * 60_000}).to_string(),
    )
    .unwrap();
    fs::write(
        state.join("usage/main.json"),
        json!({"source": "/usage", "fetched_at_ms": now - 7 * 60_000, "limits": []}).to_string(),
    )
    .unwrap();
    let out = fx.ok(&["manage", "service", "status"]);
    assert!(
        out.contains(&format!("installed: yes (~/{PLIST})")),
        "{out}"
    );
    assert!(out.contains("loaded: yes\n"), "{out}");
    assert!(out.contains(" (3m ago)\n"), "{out}");
    // No fetch before: the empty window list gives the maximum interval
    // (30m), so main is due in about 23m.
    assert!(out.contains("  main    7m old, next fetch: "), "{out}");
    assert!(out.contains(" (in 22m, every 30m)\n"), "{out}");
    assert!(out.contains("  angel2  none, next fetch: now\n"), "{out}");
    assert!(!out.contains("error:"), "{out}");
}

#[test]
fn status_and_doctor_fail_when_the_claude_path_is_gone() {
    let fx = Fixture::new("systemd");
    let moved = fx.fake.path().join("old/claude");
    fs::create_dir_all(moved.parent().unwrap()).unwrap();
    fs::copy(Fixture::fakebin().join("claude"), &moved).unwrap();
    fs::set_permissions(&moved, fs::Permissions::from_mode(0o755)).unwrap();
    let out = fx
        .ccx(&["manage", "service", "install"])
        .env("CCX_CLAUDE_BIN", &moved)
        .output()
        .unwrap();
    assert!(out.status.success());
    let doctor = fx.run(&["manage", "doctor"]);
    let text = String::from_utf8(doctor.stdout).unwrap();
    assert!(text.contains("OK    service: "), "{text}");

    // A mise upgrade moves claude.
    fs::remove_file(&moved).unwrap();
    let out = fx.run(&["manage", "service", "status"]);
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(1), "{text}");
    let want = format!(
        "error: the job uses claude at {}, which does not exist; run `ccx manage service install` again",
        moved.display()
    );
    assert!(text.contains(&want), "{text}");

    let doctor = fx.run(&["manage", "doctor"]);
    let text = String::from_utf8(doctor.stdout).unwrap();
    assert_eq!(doctor.status.code(), Some(2), "{text}");
    assert!(
        text.contains(&format!("FAIL  service: {}", &want["error: ".len()..])),
        "{text}"
    );
}

/// The job runs the background refresh, so the lock and the 60-second rule
/// of #11 apply: a job run never fetches while a different fetch holds the
/// lock, and never within 60 seconds of the last fetch.
#[test]
fn the_job_command_obeys_the_refresh_lock_and_the_gap() {
    let fx = Fixture::new("systemd");
    fx.ok(&["manage", "service", "install"]);
    let service = fs::read_to_string(fx.home().join(UNIT_DIR).join("ccx-refresh.service")).unwrap();
    let mut env = Vec::new();
    for line in service.lines() {
        if let Some(rest) = line.strip_prefix("Environment=\"") {
            let (k, v) = rest.strip_suffix('"').unwrap().split_once('=').unwrap();
            env.push((k.to_string(), v.to_string()));
        }
    }
    let exec = service
        .lines()
        .find_map(|l| l.strip_prefix("ExecStart=\""))
        .unwrap();
    let (exe, args) = exec.split_once("\" ").unwrap();
    let job = || {
        let mut cmd = Command::new(exe);
        cmd.args(args.split(' '))
            .current_dir(fx.home())
            .env_clear()
            .env("HOME", fx.home())
            .env("FAKE_CLAUDE_LOG", fx.fake.path().join("claude.log"))
            .envs(env.iter().map(|(k, v)| (k, v)));
        assert!(cmd.status().unwrap().success());
    };

    // A launch-time fetch holds the lock: the job does not fetch.
    let state = fx.home().join(".local/state/ccx");
    fs::create_dir_all(&state).unwrap();
    let lock: File = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(state.join("refresh.lock"))
        .unwrap();
    lock.lock().unwrap();
    job();
    assert_eq!(fx.usage_calls(), 0);
    drop(lock);
    // A child process that a different test starts can hold a copy of the
    // lock file for a short time, until it runs its program. Wait until the
    // lock is free, so that the job does not see it as held.
    let free = OpenOptions::new()
        .write(true)
        .open(state.join("refresh.lock"))
        .unwrap();
    free.lock().unwrap();
    drop(free);

    // The lock is free: the job fetches each account.
    job();
    assert_eq!(fx.usage_calls(), 2);
    assert!(state.join("usage/main.json").is_file());

    // Less than 60 seconds later: no fetch.
    job();
    assert_eq!(fx.usage_calls(), 2);
}

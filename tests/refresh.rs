//! End-to-end tests for stale-while-revalidate (#11): the blocking fetch of
//! an automatic launch, the detached background refresh, the refresh lock
//! and the 60-second rule. Each test uses a temp HOME and
//! `testdata/fakebin/claude`. A config dir name with "slow" makes the fake
//! `/usage` fetch sleep for 30 seconds. No test touches a real account.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

const CCX: &str = env!("CARGO_BIN_EXE_ccx");

const CONFIG: &str = r#"
[accounts.main]
dir = "~/.claude"

[accounts.angel2]
dir = "~/.claude-angel2"
"#;

const SLOW_CONFIG: &str = r#"
[accounts.main]
dir = "~/.claude"

[accounts.slow]
dir = "~/.claude-slow"
"#;

const LATER: &str = "2099-01-01T00:00:00Z";
const MINUTE: u64 = 60_000;

struct Fixture {
    home: tempfile::TempDir,
}

impl Fixture {
    fn new(config: &str) -> Fixture {
        let home = tempfile::tempdir().expect("temp HOME");
        let dir = home.path().join(".config/ccx");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.toml"), config).unwrap();
        fs::create_dir(home.path().join(".claude")).unwrap();
        Fixture { home }
    }

    fn home(&self) -> PathBuf {
        fs::canonicalize(self.home.path()).unwrap()
    }

    fn state(&self, name: &str) -> PathBuf {
        self.home().join(".local/state/ccx").join(name)
    }

    fn cache_path(&self, name: &str) -> PathBuf {
        self.state(&format!("usage/{name}.json"))
    }

    fn ccx(&self, args: &[&str]) -> Command {
        let fakebin = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fakebin");
        let mut cmd = Command::new(CCX);
        cmd.args(args)
            .current_dir(self.home())
            .env_clear()
            .env("HOME", self.home())
            .env("PATH", format!("{}:/usr/bin:/bin", fakebin.display()))
            .env("FAKE_CLAUDE_LOG", self.home().join("claude.log"));
        cmd
    }

    /// Writes a ccx cache file with this session percent and age.
    fn cache(&self, name: &str, session: f64, age_ms: u64) {
        let path = self.cache_path(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let snapshot = json!({
            "source": "/usage",
            "fetched_at_ms": now_ms() - age_ms,
            "limits": [{"kind": "session", "percent": session, "resets_at": LATER, "scope": null}],
        });
        fs::write(path, snapshot.to_string()).unwrap();
    }

    fn cached_session(&self, name: &str) -> Option<f64> {
        let value: Value = serde_json::from_slice(&fs::read(self.cache_path(name)).ok()?).ok()?;
        value["limits"][0]["percent"].as_f64()
    }

    /// Writes `cachedUsageUtilization` (the fallback) of an account.
    fn fallback(&self, account_dir: &str, session: f64, age_ms: u64) {
        let path = self.home().join(account_dir).join(".claude.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let value = json!({"cachedUsageUtilization": {"fetchedAtMs": now_ms() - age_ms,
            "utilization": {"five_hour": {"utilization": session, "resets_at": LATER}}}});
        fs::write(path, value.to_string()).unwrap();
    }

    /// Records a fetch start time of now, so that a background refresh of
    /// this launch exits with no fetch.
    fn record_start(&self) {
        fs::create_dir_all(self.state("")).unwrap();
        let text = json!({"started_at_ms": now_ms()}).to_string();
        fs::write(self.state("refresh.json"), text).unwrap();
    }

    fn calls(&self) -> Vec<Value> {
        fs::read_to_string(self.home().join("claude.log"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn usage_calls(&self) -> usize {
        self.calls()
            .iter()
            .filter(|c| c["argv"][1] == "/usage")
            .count()
    }

    fn open_lock(&self) -> File {
        fs::create_dir_all(self.state("")).unwrap();
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.state("refresh.lock"))
            .unwrap()
    }

    /// Holds the refresh lock in a different thread for `time`. Returns
    /// when the lock is held.
    fn hold_lock(&self, time: Duration) -> JoinHandle<()> {
        let file = self.open_lock();
        let (sender, receiver) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            file.lock().unwrap();
            sender.send(()).unwrap();
            std::thread::sleep(time);
            drop(file);
        });
        receiver.recv().unwrap();
        handle
    }

    /// Waits until no process holds the refresh lock.
    fn wait_for_free_lock(&self) {
        let file = self.open_lock();
        let deadline = Instant::now() + Duration::from_secs(20);
        while file.try_lock().is_err() {
            assert!(Instant::now() < deadline, "the refresh lock is still held");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Runs an automatic launch. Returns the account, the time and the output.
fn launch(cmd: &mut Command) -> (String, Duration, Output) {
    let start = Instant::now();
    let out = cmd.output().expect("run ccx");
    let elapsed = start.elapsed();
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let json: Value = serde_json::from_slice(&out.stdout).expect("fake claude JSON");
    let active = json["env"]["CCX_ACTIVE"].as_str().unwrap().to_string();
    (active, elapsed, out)
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_fresh_cache_launch_does_not_wait_for_a_fetch() {
    let fx = Fixture::new(SLOW_CONFIG);
    fx.cache("main", 50.0, 4 * MINUTE);
    fx.cache("slow", 10.0, 14 * MINUTE);
    fx.record_start();
    let (active, elapsed, _) = launch(&mut fx.ccx(&[]));
    assert_eq!(active, "slow");
    assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
    assert_eq!(fx.usage_calls(), 0);
}

#[test]
fn the_background_refresh_continues_after_the_exec_with_no_output() {
    let fx = Fixture::new(CONFIG);
    fx.cache("main", 50.0, 4 * MINUTE);
    fx.cache("angel2", 50.0, 4 * MINUTE);
    let limits = json!([{"kind": "session", "percent": 42, "resets_at": LATER, "scope": null}]);
    let (_, elapsed, out) = launch(
        fx.ccx(&[])
            .env("FAKE_USAGE_SLEEP", "1")
            .env("FAKE_USAGE_LIMITS", limits.to_string()),
    );
    // ccx did not wait for the refresh, and the refresh wrote nothing to
    // the output of the launch. `output()` returns only when all copies of
    // the pipes close.
    assert!(elapsed < Duration::from_secs(1), "{elapsed:?}");
    assert_eq!(stderr(&out), "");
    assert_eq!(fx.cached_session("main"), Some(50.0));
    // The refresh finishes after claude exits.
    wait_until("the background refresh", || {
        fx.cached_session("main") == Some(42.0) && fx.cached_session("angel2") == Some(42.0)
    });
    fx.wait_for_free_lock();
    assert_eq!(fx.usage_calls(), 2);
    let argv: Vec<Value> = fx
        .calls()
        .iter()
        .filter(|c| c["argv"][1] != "/usage")
        .map(|c| c["argv"].clone())
        .collect();
    assert_eq!(argv, [json!([])], "only the launch is not a fetch");
    assert!(fx.state("refresh.json").exists());
}

#[test]
fn background_refreshes_use_the_lock_and_the_60_second_rule() {
    let fx = Fixture::new(CONFIG);
    let background = ["usage", "refresh", "--all", "--background"];

    // A different process holds the lock: the background refresh exits.
    let (release, wait) = mpsc::channel::<()>();
    let file = fx.open_lock();
    file.lock().unwrap();
    let holder = std::thread::spawn(move || {
        wait.recv().unwrap();
        drop(file);
    });
    let start = Instant::now();
    let out = fx.ccx(&background).output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(start.elapsed() < Duration::from_secs(3));
    assert_eq!(out.stdout, b"");
    assert_eq!(fx.usage_calls(), 0);
    assert!(!fx.cache_path("main").exists());
    release.send(()).unwrap();
    holder.join().unwrap();

    // The lock is free and no fetch started before: the refresh runs and
    // writes nothing.
    let out = fx.ccx(&background).output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        (out.stdout.as_slice(), out.stderr.as_slice()),
        (&b""[..], &b""[..])
    );
    assert_eq!(fx.usage_calls(), 2);
    assert!(fx.cache_path("main").exists() && fx.cache_path("angel2").exists());

    // Less than 60 seconds after the last start: no fetch.
    let out = fx.ccx(&background).output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(fx.usage_calls(), 2);

    // A manual refresh does not use the 60-second rule.
    let out = fx.ccx(&["usage", "refresh", "--all"]).output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(fx.usage_calls(), 4);
}

#[test]
fn a_manual_refresh_waits_for_the_lock_and_then_runs() {
    let fx = Fixture::new(CONFIG);
    fx.record_start();
    let holder = fx.hold_lock(Duration::from_millis(1500));
    let start = Instant::now();
    let out = fx.ccx(&["usage", "refresh", "main"]).output().unwrap();
    let elapsed = start.elapsed();
    holder.join().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(elapsed >= Duration::from_millis(1400), "{elapsed:?}");
    assert_eq!(fx.usage_calls(), 1);
    assert!(fx.cache_path("main").exists());
}

#[test]
fn a_slow_blocking_fetch_stops_after_5_seconds_and_uses_the_fallback() {
    let fx = Fixture::new(SLOW_CONFIG);
    fx.cache("main", 50.0, 4 * MINUTE);
    // Only the fallback of `slow` is known, and it is one hour old.
    fx.fallback(".claude-slow", 1.0, 60 * MINUTE);
    let (active, elapsed, _) = launch(&mut fx.ccx(&[]));
    assert!(elapsed >= Duration::from_millis(4800), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(9), "{elapsed:?}");
    // With unknown usage, `main` wins the tie; the fallback selects `slow`.
    assert_eq!(active, "slow");
    assert_eq!(fx.usage_calls(), 1, "only `slow` was stale");
    assert!(!fx.cache_path("slow").exists());
}

#[test]
fn a_blocking_fetch_stops_after_5_seconds_when_the_lock_is_held() {
    let fx = Fixture::new(CONFIG);
    fx.cache("main", 50.0, 4 * MINUTE);
    fx.fallback(".claude-angel2", 1.0, 60 * MINUTE);
    fx.record_start();
    let holder = fx.hold_lock(Duration::from_secs(7));
    let (active, elapsed, _) = launch(&mut fx.ccx(&[]));
    assert!(elapsed >= Duration::from_millis(4800), "{elapsed:?}");
    assert!(elapsed < Duration::from_millis(6900), "{elapsed:?}");
    assert_eq!(active, "angel2", "the fallback selects angel2");
    holder.join().unwrap();
    assert_eq!(fx.usage_calls(), 0);
}

#[test]
fn a_blocking_fetch_runs_when_the_lock_is_released_in_time() {
    let fx = Fixture::new(CONFIG);
    fx.cache("main", 50.0, 4 * MINUTE);
    fx.fallback(".claude-angel2", 1.0, 60 * MINUTE);
    let holder = fx.hold_lock(Duration::from_secs(1));
    let limits = json!([{"kind": "session", "percent": 80, "resets_at": LATER, "scope": null}]);
    let (active, elapsed, _) = launch(fx.ccx(&[]).env("FAKE_USAGE_LIMITS", limits.to_string()));
    holder.join().unwrap();
    assert!(elapsed >= Duration::from_millis(900), "{elapsed:?}");
    assert!(elapsed < Duration::from_millis(4500), "{elapsed:?}");
    // The new data (80%) makes `main` (50%) the best account.
    assert_eq!(active, "main");
    assert_eq!(fx.cached_session("angel2"), Some(80.0));
    // The blocking fetch recorded its start, so the background refresh of
    // this launch exits with no fetch.
    fx.wait_for_free_lock();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(fx.usage_calls(), 1);
}

#[test]
fn a_named_launch_fetches_nothing_and_starts_no_refresh() {
    let fx = Fixture::new(CONFIG);
    let (active, _, _) = launch(&mut fx.ccx(&["@angel2"]));
    assert_eq!(active, "angel2");
    std::thread::sleep(Duration::from_millis(1000));
    assert_eq!(fx.calls().len(), 1, "only the launch");
    assert!(!fx.state("refresh.lock").exists());
    assert!(!fx.state("refresh.json").exists());
}

#[test]
fn ccx_no_refresh_turns_off_the_launch_fetches() {
    let fx = Fixture::new(CONFIG);
    let (_, _, _) = launch(fx.ccx(&[]).env("CCX_NO_REFRESH", "1"));
    std::thread::sleep(Duration::from_millis(1000));
    assert_eq!(fx.calls().len(), 1, "only the launch");
    assert!(!fx.state("refresh.lock").exists());
}

//! End-to-end tests for automatic selection (#10). Each test uses a temp
//! HOME, synthetic usage cache files and `testdata/fakebin/claude`, which
//! prints its argv and env as JSON. No test touches a real account.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

const CCX: &str = env!("CARGO_BIN_EXE_ccx");

const CONFIG: &str = r#"
[accounts.main]
dir = "~/.claude"

[accounts.angel2]
dir = "~/.claude-angel2"
"#;

/// A reset time far in the future, so that no window has reset.
const LATER: &str = "2099-01-01T00:00:00Z";
const MUCH_LATER: &str = "2099-06-01T00:00:00Z";

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

    fn log(&self) -> PathBuf {
        self.home().join("claude.log")
    }

    fn state(&self) -> PathBuf {
        self.home().join(".local/state/ccx/state.json")
    }

    fn ccx(&self, args: &[&str]) -> Command {
        let fakebin = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fakebin");
        let mut cmd = Command::new(CCX);
        cmd.args(args)
            .current_dir(self.home())
            .env_clear()
            .env("HOME", self.home())
            .env("PATH", format!("{}:/usr/bin:/bin", fakebin.display()))
            .env("FAKE_CLAUDE_LOG", self.log());
        cmd
    }

    /// Writes a usage cache file for `name` with these windows.
    fn cache(&self, name: &str, limits: Value) {
        let path = self
            .home()
            .join(format!(".local/state/ccx/usage/{name}.json"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let snapshot = json!({
            "source": "/usage",
            "fetched_at_ms": now_ms() - 4 * 60_000,
            "limits": limits,
        });
        fs::write(path, snapshot.to_string()).unwrap();
    }

    /// Writes `.claude.json` of an account (`~` is the base account).
    fn claude_json(&self, account_dir: &str, value: Value) {
        let path = if account_dir == "~" {
            self.home().join(".claude.json")
        } else {
            self.home().join(account_dir).join(".claude.json")
        };
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, value.to_string()).unwrap();
    }

    fn calls(&self) -> usize {
        fs::read_to_string(self.log())
            .unwrap_or_default()
            .lines()
            .count()
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn usage(session: f64, weekly: f64) -> Value {
    json!([
        {"kind": "session", "percent": session, "resets_at": LATER, "scope": null},
        {"kind": "weekly_all", "percent": weekly, "resets_at": MUCH_LATER, "scope": null},
    ])
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Runs ccx and returns the JSON that the fake claude prints.
fn launched(cmd: &mut Command) -> (Value, Output) {
    let out = cmd.output().expect("run ccx");
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let json = serde_json::from_slice(&out.stdout).expect("fake claude JSON");
    (json, out)
}

fn active(fx: &Fixture, args: &[&str]) -> String {
    let (json, _) = launched(&mut fx.ccx(args));
    json["env"]["CCX_ACTIVE"].as_str().unwrap().to_string()
}

/// Runs ccx on a pseudo-terminal with `script`, so that stdout and stderr
/// are a TTY. Returns all output (stdout and stderr together).
fn on_tty(cmd: &Command, args: &[&str]) -> String {
    let mut script = Command::new("script");
    if cfg!(target_os = "macos") {
        script.args(["-q", "/dev/null", CCX]).args(args);
    } else {
        let line = std::iter::once(CCX)
            .chain(args.iter().copied())
            .map(|a| format!("'{a}'"))
            .collect::<Vec<_>>()
            .join(" ");
        script.args(["-qec", &line, "/dev/null"]);
    }
    script.env_clear();
    if let Some(dir) = cmd.get_current_dir() {
        script.current_dir(dir);
    }
    for (key, value) in cmd.get_envs() {
        if let Some(value) = value {
            script.env(key, value);
        }
    }
    let out = script.output().expect("run script");
    assert!(out.status.success(), "script failed: {}", stderr(&out));
    String::from_utf8_lossy(&out.stdout).replace('\r', "")
}

#[test]
fn launches_the_expected_account_with_its_config_dir() {
    let fx = Fixture::new(CONFIG);
    fx.cache("main", usage(92.0, 10.0));
    fx.cache("angel2", usage(10.0, 20.0));
    let (json, out) = launched(&mut fx.ccx(&["--model", "opus"]));
    let want_dir = format!("{}/.claude-angel2", fx.home().display());
    assert_eq!(json["env"]["CLAUDE_CONFIG_DIR"], want_dir.as_str());
    assert_eq!(json["env"]["CCX_ACTIVE"], "angel2");
    assert_eq!(json["argv"], json!(["--model", "opus"]));
    // No banner and no fetch: the fake claude ran once, for the launch.
    assert_eq!(stderr(&out), "");
    assert_eq!(fx.calls(), 1);

    // The base account wins when it has more capacity.
    fx.cache("main", usage(5.0, 10.0));
    let (json, _) = launched(&mut fx.ccx(&[]));
    assert!(json["env"].get("CLAUDE_CONFIG_DIR").is_none());
    assert_eq!(json["env"]["CCX_ACTIVE"], "main");
}

#[test]
fn the_pick_is_written_to_state_json_at_mode_0600() {
    let fx = Fixture::new(CONFIG);
    fx.cache("main", usage(92.0, 10.0));
    assert_eq!(active(&fx, &[]), "angel2");
    let state: Value = serde_json::from_slice(&fs::read(fx.state()).unwrap()).unwrap();
    assert_eq!(state["last"], "angel2");
    assert!(state["picked_at_ms"].as_u64().unwrap() > 0);
    let mode = fs::metadata(fx.state()).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let names: Vec<_> = fs::read_dir(fx.state().parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(names.is_empty(), "{names:?}");
}

#[test]
fn named_launches_and_subcommands_do_not_use_the_picker() {
    let fx = Fixture::new(CONFIG);
    fx.cache("main", usage(99.0, 99.0));
    assert_eq!(active(&fx, &["@main"]), "main");
    assert_eq!(active(&fx, &["auth", "status"]), "main");
    let (json, _) = launched(fx.ccx(&[]).env("CCX_ACCOUNT", "main"));
    assert_eq!(json["env"]["CCX_ACTIVE"], "main");
    assert!(!fx.state().exists());
}

#[test]
fn weights_come_from_the_tier_and_the_config() {
    let fx = Fixture::new(CONFIG);
    let tier = |org: &str, tier: &str| json!({"oauthAccount": {"organizationType": org, "organizationRateLimitTier": tier}});
    fx.claude_json("~", tier("claude_max", "default_claude_max_5x"));
    fx.claude_json(
        ".claude-angel2",
        tier("claude_max", "default_claude_max_20x"),
    );
    // Unknown usage: 50 × 5 against 50 × 20.
    assert_eq!(active(&fx, &[]), "angel2");
    // Session 60% on angel2: 40 × 20 = 800 is still more than 250.
    fx.cache("angel2", usage(60.0, 0.0));
    assert_eq!(active(&fx, &[]), "angel2");

    let fx = Fixture::new(&CONFIG.replace(
        "dir = \"~/.claude\"\n",
        "dir = \"~/.claude\"\nweight = 100\n",
    ));
    fx.claude_json(
        ".claude-angel2",
        tier("claude_max", "default_claude_max_20x"),
    );
    assert_eq!(active(&fx, &[]), "main");
}

#[test]
fn cached_usage_utilization_is_the_fallback() {
    let fx = Fixture::new(CONFIG);
    let cached = |percent: f64| {
        json!({"cachedUsageUtilization": {"fetchedAtMs": now_ms() - 60_000, "utilization": {
            "five_hour": {"utilization": percent, "resets_at": LATER},
            "seven_day": {"utilization": 1, "resets_at": MUCH_LATER}
        }}})
    };
    fx.claude_json("~", cached(95.0));
    fx.claude_json(".claude-angel2", cached(10.0));
    assert_eq!(active(&fx, &[]), "angel2");
}

#[test]
fn the_last_account_stays_when_it_is_near_the_best() {
    let fx = Fixture::new(CONFIG);
    fx.cache("main", usage(0.0, 0.0));
    fx.cache("angel2", usage(8.0, 0.0));
    assert_eq!(active(&fx, &[]), "main");
    let state = fx.state();
    fs::write(&state, r#"{"last": "angel2"}"#).unwrap();
    assert_eq!(active(&fx, &[]), "angel2");
    // A difference of more than 10 points changes the account.
    fx.cache("angel2", usage(11.0, 0.0));
    assert_eq!(active(&fx, &[]), "main");
}

#[test]
fn a_matched_scoped_limit_avoids_the_account() {
    let fx = Fixture::new(CONFIG);
    fx.cache("main", usage(50.0, 0.0));
    fx.cache(
        "angel2",
        json!([
            {"kind": "session", "percent": 0, "resets_at": null, "scope": null},
            {"kind": "weekly_scoped", "percent": 99, "resets_at": LATER, "scope": "Opus"},
        ]),
    );
    assert_eq!(active(&fx, &[]), "angel2");
    assert_eq!(active(&fx, &["--model", "claude-opus-4"]), "main");
    assert_eq!(active(&fx, &["--model=sonnet"]), "angel2");
    // The `model` in settings.json applies when there is no `--model`.
    let settings = fx.home().join(".claude-angel2/settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, r#"{"model": "opus"}"#).unwrap();
    assert_eq!(active(&fx, &[]), "main");
}

#[test]
fn all_limited_launches_the_soonest_reset_with_a_warning() {
    let fx = Fixture::new(CONFIG);
    fx.cache(
        "main",
        json!([{"kind": "session", "percent": 95, "resets_at": MUCH_LATER, "scope": null}]),
    );
    fx.cache(
        "angel2",
        json!([{"kind": "weekly_all", "percent": 99, "resets_at": LATER, "scope": null}]),
    );
    let (json, out) = launched(&mut fx.ccx(&[]));
    assert_eq!(json["env"]["CCX_ACTIVE"], "angel2");
    // No warning in a pipe.
    assert_eq!(stderr(&out), "");

    let output = on_tty(&fx.ccx(&[]), &[]);
    assert_eq!(
        output
            .matches("ccx: warning: all accounts limited; angel2 frees ")
            .count(),
        1,
        "{output}"
    );
    assert!(output.contains("\"CCX_ACTIVE\":\"angel2\""), "{output}");
}

#[test]
fn unknown_usage_never_blocks_a_launch() {
    let fx = Fixture::new(CONFIG);
    fx.claude_json("~", json!({"cachedUsageUtilization": "not an object"}));
    fs::create_dir_all(fx.home().join(".local/state/ccx/usage")).unwrap();
    fs::write(fx.home().join(".local/state/ccx/usage/main.json"), "{bad").unwrap();
    fs::write(fx.state(), "not json").unwrap();
    let (json, _) = launched(&mut fx.ccx(&[]));
    assert!(json["env"]["CCX_ACTIVE"].is_string());
}

#[test]
fn the_banner_shows_only_on_a_tty_and_not_for_print_runs() {
    let fx = Fixture::new(CONFIG);
    fx.cache("main", usage(92.0, 10.0));
    fx.cache("angel2", usage(10.4, 20.0));
    let output = on_tty(&fx.ccx(&[]), &[]);
    // `script` on macOS can put `^D` before the first line.
    let banner = output
        .lines()
        .find_map(|l| l.find("ccx → ").map(|i| &l[i..]))
        .unwrap_or_else(|| panic!("no banner: {output}"));
    assert!(
        banner.starts_with("ccx → angel2 · 5h 10% · wk 20% (resets "),
        "{banner}"
    );
    assert!(banner.ends_with(") · /usage 4m"), "{banner}");

    for args in [&["-p", "hi"][..], &["--print", "hi"]] {
        let output = on_tty(&fx.ccx(&[]), args);
        assert!(!output.contains("ccx →"), "{args:?}: {output}");
        assert!(output.contains("\"argv\""), "{output}");
    }

    // In a pipe there is no banner.
    let (_, out) = launched(&mut fx.ccx(&[]));
    assert_eq!(stderr(&out), "");

    // `banner = false` turns it off.
    let fx = Fixture::new(&format!("[launch]\nbanner = false\n{CONFIG}"));
    let output = on_tty(&fx.ccx(&[]), &[]);
    assert!(!output.contains("ccx →"), "{output}");
}

//! End-to-end tests for `ccx status` and `ccx usage refresh`. Each test
//! uses a temp HOME and `testdata/fakebin/claude`, which logs its argv,
//! cwd and env and prints synthetic `/usage` output. No test touches a real
//! account.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const CCX: &str = env!("CARGO_BIN_EXE_ccx");

const USAGE_ARGV: [&str; 9] = [
    "-p",
    "/usage",
    "--output-format",
    "stream-json",
    "--verbose",
    "--no-session-persistence",
    "--strict-mcp-config",
    "--settings",
    r#"{"disableAllHooks":true}"#,
];

const CONFIG: &str = r#"
[accounts.main]
dir = "~/.claude"

[accounts.angel2]
dir = "~/.claude-angel2"
"#;

struct Fixture {
    home: tempfile::TempDir,
}

impl Fixture {
    fn new(config: &str) -> Fixture {
        let home = tempfile::tempdir().expect("temp HOME");
        let dir = home.path().join(".config/ccx");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.toml"), config).unwrap();
        if config.contains("[accounts.angel2]") {
            fs::write(
                dir.join("config.local.toml"),
                "[accounts.angel2]\nemail = \"work@example.com\"\n",
            )
            .unwrap();
        }
        Fixture { home }
    }

    /// The physical HOME path. On macOS the temp dir is behind a symlink,
    /// and the fake claude reports the physical cwd.
    fn home(&self) -> PathBuf {
        fs::canonicalize(self.home.path()).unwrap()
    }

    fn log(&self) -> PathBuf {
        self.home().join("claude.log")
    }

    fn probe(&self) -> PathBuf {
        self.home().join(".local/state/ccx/probe")
    }

    fn cache(&self, name: &str) -> PathBuf {
        self.home()
            .join(format!(".local/state/ccx/usage/{name}.json"))
    }

    fn ccx(&self, args: &[&str]) -> Command {
        let fakebin = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fakebin");
        let mut cmd = Command::new(CCX);
        cmd.args(args)
            .current_dir(self.home())
            .env_clear()
            .env("HOME", self.home())
            .env("PATH", format!("{}:/usr/bin:/bin", fakebin.display()))
            .env("FAKE_CLAUDE_LOG", self.log())
            .env("ANTHROPIC_API_KEY", "sk-test-not-a-key")
            .env("CLAUDE_CONFIG_DIR", "/should/be/replaced");
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.ccx(args).output().expect("run ccx")
    }

    /// The fake claude calls, one JSON object for each call.
    fn calls(&self) -> Vec<Value> {
        fs::read_to_string(self.log())
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn usage_calls(&self) -> Vec<Value> {
        self.calls()
            .into_iter()
            .filter(|c| c["argv"][1] == "/usage")
            .collect()
    }

    fn write_claude_json(&self, account_dir: &str, value: Value) {
        let path = if account_dir == "~" {
            self.home().join(".claude.json")
        } else {
            self.home().join(account_dir).join(".claude.json")
        };
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, value.to_string()).unwrap();
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn status_json(fx: &Fixture) -> (Value, Option<i32>) {
    let out = fx.run(&["status", "--json"]);
    let value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}{}", stdout(&out), stderr(&out)));
    (value, out.status.code())
}

fn account<'a>(status: &'a Value, name: &str) -> &'a Value {
    status["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == name)
        .unwrap()
}

#[test]
fn refresh_runs_the_exact_command_env_and_cwd() {
    let fx = Fixture::new(CONFIG);
    let out = fx.run(&["usage", "refresh", "angel2"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), "angel2: ok (3 usage windows)\n");

    let calls = fx.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call["argv"], json!(USAGE_ARGV));
    assert_eq!(call["cwd"], fx.probe().to_str().unwrap());
    let env = &call["env"];
    assert_eq!(
        env["CLAUDE_CONFIG_DIR"],
        fx.home().join(".claude-angel2").to_str().unwrap()
    );
    assert_eq!(env["ANTHROPIC_API_KEY"], Value::Null);
    assert_eq!(env["HOME"], fx.home().to_str().unwrap());
}

#[test]
fn refresh_of_the_base_account_unsets_claude_config_dir() {
    let fx = Fixture::new(CONFIG);
    let out = fx.run(&["usage", "refresh", "main"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let call = &fx.calls()[0];
    assert_eq!(call["argv"], json!(USAGE_ARGV));
    assert_eq!(call["cwd"], fx.probe().to_str().unwrap());
    assert_eq!(call["env"]["CLAUDE_CONFIG_DIR"], Value::Null);
}

#[test]
fn refresh_never_runs_in_the_project_directory() {
    let fx = Fixture::new(CONFIG);
    let project = fx.home().join("project");
    fs::create_dir_all(project.join(".claude")).unwrap();
    fs::write(
        project.join(".claude/settings.json"),
        r#"{"hooks": {"SessionStart": []}}"#,
    )
    .unwrap();
    fs::write(project.join(".mcp.json"), r#"{"mcpServers": {}}"#).unwrap();
    fs::write(project.join("CLAUDE.md"), "project rules\n").unwrap();

    let out = fx
        .ccx(&["usage", "refresh", "--all"])
        .current_dir(&project)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let calls = fx.usage_calls();
    assert_eq!(calls.len(), 2);
    for call in calls {
        assert_eq!(call["cwd"], fx.probe().to_str().unwrap());
        assert_ne!(call["cwd"], project.to_str().unwrap());
    }
}

#[test]
fn refresh_makes_the_probe_dir_0700_and_the_cache_0600() {
    let fx = Fixture::new(CONFIG);
    assert!(!fx.probe().exists());
    let out = fx.run(&["usage", "refresh", "--all"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(mode(&fx.probe()), 0o700);
    for name in ["main", "angel2"] {
        let cache = fx.cache(name);
        assert_eq!(mode(&cache), 0o600, "{name}");
        let value: Value = serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
        assert_eq!(value["source"], "/usage");
        assert_eq!(value["limits"].as_array().unwrap().len(), 3);
        assert_eq!(value["limits"][0]["resets_at"], Value::Null);
    }
    // No temp files stay in the cache dir.
    let names: Vec<_> = fs::read_dir(fx.cache("main").parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
}

#[test]
fn refresh_reports_a_failed_fetch_and_an_unknown_name() {
    let fx = Fixture::new(&format!(
        "{CONFIG}\n[accounts.nousage]\ndir = \"~/.claude-nousage\"\n"
    ));
    let out = fx.run(&["usage", "refresh", "nousage"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("ccx: nousage: usage fetch:"),
        "{}",
        stderr(&out)
    );
    assert!(!fx.cache("nousage").exists());

    let out = fx.run(&["usage", "refresh", "ghost"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("unknown account `ghost`"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn status_fetches_writes_the_cache_and_then_uses_it() {
    let fx = Fixture::new(CONFIG);
    fx.write_claude_json(
        "~",
        json!({"oauthAccount": {"organizationRateLimitTier": "default_claude_max_20x"}}),
    );
    fx.write_claude_json(
        ".claude-angel2",
        json!({"oauthAccount": {"organizationRateLimitTier": "future_tier_xyz"}}),
    );
    let (status, code) = status_json(&fx);
    assert_eq!(code, Some(0));
    assert_eq!(status["exit_code"], 0);
    let angel2 = account(&status, "angel2");
    assert_eq!(angel2["email"], "work@example.com");
    assert_eq!(angel2["logged_in"], true);
    assert_eq!(angel2["subscription_type"], "max");
    assert_eq!(angel2["tier"], "future_tier_xyz");
    assert_eq!(angel2["source"], "/usage");
    assert_eq!(
        angel2["session"],
        json!({"percent": 5.0, "resets_at": null})
    );
    assert_eq!(angel2["weekly"]["percent"], 11.0);
    assert_eq!(angel2["scoped"][0]["model"], "ExampleModel");
    let main = account(&status, "main");
    assert_eq!(main["tier"], "Max 20x");
    assert_eq!(main["email"], Value::Null);
    assert!(fx.cache("main").exists() && fx.cache("angel2").exists());
    // Each usage call ran in the probe dir with the right account dir.
    let calls = fx.usage_calls();
    assert_eq!(calls.len(), 2);
    for call in &calls {
        assert_eq!(call["argv"], json!(USAGE_ARGV));
        assert_eq!(call["cwd"], fx.probe().to_str().unwrap());
    }
    // `claude auth status` also runs in the probe dir.
    for call in fx.calls().iter().filter(|c| c["argv"][0] == "auth") {
        assert_eq!(call["cwd"], fx.probe().to_str().unwrap());
    }

    // The cache is fresh now: a second status fetches nothing.
    let (status, _) = status_json(&fx);
    assert_eq!(fx.usage_calls().len(), 2);
    assert_eq!(account(&status, "angel2")["source"], "/usage");
}

#[test]
fn status_text_has_one_row_for_each_account() {
    let fx = Fixture::new(CONFIG);
    let out = fx.run(&["status"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    assert!(lines[0].starts_with("NAME"), "{text}");
    for word in [
        "EMAIL", "KIND", "TIER", "SESSION", "WEEKLY", "SCOPED", "SOURCE", "AGE",
    ] {
        assert!(lines[0].contains(word), "{word}: {text}");
    }
    assert!(lines[1].starts_with("angel2"), "{text}");
    assert!(lines[1].contains("work@example.com"), "{text}");
    assert!(lines[1].contains("ExampleModel 0%"), "{text}");
    assert!(lines[1].contains("/usage"), "{text}");
    assert!(lines[2].starts_with("main"), "{text}");
}

#[test]
fn failed_fetch_uses_cached_usage_utilization_or_shows_unknown() {
    let fx = Fixture::new(
        r#"
[accounts.main]
dir = "~/.claude"

[accounts.nousage]
dir = "~/.claude-nousage"

[accounts.badjson]
dir = "~/.claude-badjson"
"#,
    );
    // main: the fetch works. nousage: the fetch fails and the fallback is
    // present. badjson: the fetch gives bad JSON and there is no fallback.
    let fetched_at = 1_767_225_600_000u64;
    fx.write_claude_json(
        ".claude-nousage",
        json!({"cachedUsageUtilization": {"fetchedAtMs": fetched_at, "utilization": {
            "limits": [{"kind": "session", "percent": 42, "resets_at": null, "scope": null}]
        }}}),
    );
    let (status, code) = status_json(&fx);
    assert_eq!(code, Some(0));

    let fallback = account(&status, "nousage");
    assert_eq!(fallback["source"], "cachedUsageUtilization");
    assert_eq!(fallback["fetched_at_ms"], fetched_at);
    assert!(fallback["age_secs"].as_u64().unwrap() > 0);
    assert_eq!(fallback["session"]["percent"], 42.0);
    assert!(fallback["error"].as_str().unwrap().contains("failed"));
    assert!(!fx.cache("nousage").exists());

    let unknown = account(&status, "badjson");
    assert_eq!(unknown["source"], Value::Null);
    assert_eq!(unknown["session"], Value::Null);
    assert!(unknown["error"].is_string());

    let out = fx.run(&["status"]);
    assert!(out.status.success());
    let text = stdout(&out);
    let row = text.lines().find(|l| l.starts_with("badjson")).unwrap();
    assert!(row.contains("unknown"), "{text}");
    let row = text.lines().find(|l| l.starts_with("nousage")).unwrap();
    assert!(row.contains("cachedUsageUtilization"), "{text}");
    assert!(row.contains("42%"), "{text}");
}

#[test]
fn a_slow_fetch_times_out_and_status_still_exits_0() {
    let fx = Fixture::new(
        r#"
[accounts.main]
dir = "~/.claude"

[accounts.slow]
dir = "~/.claude-slow"

[usage]
fetch_timeout_secs = 1
"#,
    );
    let start = Instant::now();
    let (status, code) = status_json(&fx);
    assert!(start.elapsed() < Duration::from_secs(20));
    assert_eq!(code, Some(0));
    let slow = account(&status, "slow");
    assert_eq!(slow["source"], Value::Null);
    assert!(
        slow["error"].as_str().unwrap().contains("timed out"),
        "{slow}"
    );
}

#[test]
fn exit_code_is_3_when_all_accounts_are_limited() {
    let fx = Fixture::new(CONFIG);
    let out = fx
        .ccx(&["status", "--json"])
        .env(
            "FAKE_USAGE_LIMITS",
            r#"[{"kind": "session", "percent": 93, "resets_at": null},
                {"kind": "weekly_all", "percent": 10, "resets_at": null}]"#,
        )
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    let status: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(account(&status, "main")["limited"], true);
    assert_eq!(status["exit_code"], 3);
}

#[test]
fn exit_code_is_4_when_no_account_is_logged_in() {
    let fx = Fixture::new(CONFIG);
    let out = fx
        .ccx(&["status"])
        .env("FAKE_LOGGED_OUT", "1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(4), "{}", stderr(&out));
    assert!(stdout(&out).contains("logged out"), "{}", stdout(&out));
    // ccx does not fetch usage for a logged-out account.
    assert!(fx.usage_calls().is_empty());
}

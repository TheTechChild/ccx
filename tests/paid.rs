//! End-to-end tests for API-key accounts as a paid fallback (#20). Each
//! test uses a temp HOME, synthetic usage caches, `testdata/fakebin/claude`
//! and the fake secret stores `testdata/fakebin/security` and
//! `testdata/fakebin/secret-tool`. No test touches a real account, a real
//! keychain or a real key.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

const CCX: &str = env!("CARGO_BIN_EXE_ccx");

/// Synthetic keys only.
const KEY: &str = "sk-ant-test-paid-0000";
const SHELL_KEY: &str = "sk-ant-test-shell-1111";

const LATER: &str = "2099-01-01T00:00:00Z";
const MUCH_LATER: &str = "2099-06-01T00:00:00Z";

fn config(key_lines: &str) -> String {
    format!(
        r#"[accounts.main]
dir = "~/.claude"

[accounts.angel2]
dir = "~/.claude-angel2"

[accounts.paid]
dir = "~/.claude-paid"
kind = "api_key"
{key_lines}
"#
    )
}

struct Fixture {
    home: tempfile::TempDir,
}

impl Fixture {
    /// A paid account with its key in the 0600 key file.
    fn new() -> Fixture {
        let fx = Fixture::with_config(&config("key_store = \"file\""));
        fx.write_key_file(KEY, 0o600);
        fx
    }

    fn with_config(config: &str) -> Fixture {
        let home = tempfile::tempdir().expect("temp HOME");
        let fx = Fixture { home };
        fs::create_dir_all(fx.config_dir()).unwrap();
        fs::write(fx.config_path(), config).unwrap();
        let base = fx.home().join(".claude");
        fs::create_dir_all(base.join("skills/tdd")).unwrap();
        fs::write(base.join("settings.json"), "{}\n").unwrap();
        for dir in [".claude-angel2", ".claude-paid"] {
            fs::create_dir(fx.home().join(dir)).unwrap();
        }
        fx
    }

    fn home(&self) -> PathBuf {
        fs::canonicalize(self.home.path()).unwrap()
    }

    fn config_dir(&self) -> PathBuf {
        self.home.path().join(".config/ccx")
    }

    fn config_path(&self) -> PathBuf {
        self.config_dir().join("config.toml")
    }

    fn key_file(&self) -> PathBuf {
        self.config_dir().join("keys/paid")
    }

    fn secret_item(&self) -> PathBuf {
        self.home().join(".fake-secrets/ccx-paid")
    }

    fn log(&self) -> PathBuf {
        self.home().join("claude.log")
    }

    fn write_key_file(&self, key: &str, mode: u32) {
        let dir = self.key_file().parent().unwrap().to_path_buf();
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(self.key_file(), key).unwrap();
        fs::set_permissions(self.key_file(), fs::Permissions::from_mode(mode)).unwrap();
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
            .env("CCX_NO_REFRESH", "1")
            // A key in the shell of the user. ccx must strip it.
            .env("ANTHROPIC_API_KEY", SHELL_KEY);
        cmd
    }

    /// Writes a usage cache file for `name`.
    fn cache(&self, name: &str, session: f64, weekly: f64) {
        let path = self
            .home()
            .join(format!(".local/state/ccx/usage/{name}.json"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let snapshot = json!({
            "source": "/usage",
            "fetched_at_ms": now_ms() - 4 * 60_000,
            "limits": [
                {"kind": "session", "percent": session, "resets_at": LATER, "scope": null},
                {"kind": "weekly_all", "percent": weekly, "resets_at": MUCH_LATER, "scope": null},
            ],
        });
        fs::write(path, snapshot.to_string()).unwrap();
    }

    /// All subscription accounts are limited. `main` resets first.
    fn all_limited(&self) {
        self.cache("main", 95.0, 10.0);
        self.cache("angel2", 10.0, 97.0);
    }

    /// The text of each file under HOME, except the key file and the fake
    /// secret store, which hold the key by design.
    fn all_text(&self) -> String {
        let mut out = String::new();
        let skip = [self.key_file(), self.secret_item()];
        let mut stack = vec![self.home()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                let meta = fs::symlink_metadata(&path).unwrap();
                if meta.is_dir() {
                    stack.push(path);
                } else if meta.is_file()
                    && !skip
                        .iter()
                        .any(|s| fs::canonicalize(s).ok() == Some(path.clone()))
                    && path != self.log()
                {
                    out.push_str(&String::from_utf8_lossy(&fs::read(&path).unwrap()));
                }
            }
        }
        out
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

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Runs ccx and returns the JSON that the fake claude prints.
fn launched(cmd: &mut Command) -> Value {
    let out = cmd.output().expect("run ccx");
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    serde_json::from_slice(&out.stdout).expect("fake claude JSON")
}

/// Runs ccx and expects a failure with no launch.
fn refused(cmd: &mut Command) -> String {
    let out = cmd.output().expect("run ccx");
    assert!(!out.status.success(), "stdout: {}", stdout(&out));
    assert_eq!(stdout(&out), "", "claude must not start");
    stderr(&out)
}

/// Runs ccx with `input` on stdin.
fn with_input(cmd: &mut Command, input: &str) -> Output {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run ccx");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// Runs ccx on a pseudo-terminal with `script`, so that stdin, stdout and
/// stderr are a TTY. `input` goes to the terminal. Returns all output.
fn on_tty(cmd: &Command, args: &[&str], input: &str) -> String {
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
    let out = with_input(&mut script, input);
    String::from_utf8_lossy(&out.stdout).replace('\r', "")
}

fn account_of(json: &Value) -> &str {
    json["env"]["CCX_ACTIVE"].as_str().unwrap()
}

// Selection and consent.

#[test]
fn with_no_tty_and_no_flag_ccx_refuses_the_paid_account_and_uses_the_soonest_reset() {
    let fx = Fixture::new();
    fx.all_limited();
    let json = launched(&mut fx.ccx(&["-p", "hi"]));
    assert_eq!(account_of(&json), "main");
    assert!(json["env"].get("ANTHROPIC_API_KEY").is_none(), "{json}");
}

#[test]
fn allow_paid_uses_the_paid_account_with_its_key_and_no_prompt() {
    let fx = Fixture::new();
    fx.all_limited();
    let json = launched(&mut fx.ccx(&["--allow-paid", "-p", "hi"]));
    assert_eq!(account_of(&json), "paid");
    assert_eq!(json["env"]["ANTHROPIC_API_KEY"], KEY);
    let dir = format!("{}/.claude-paid", fx.home().display());
    assert_eq!(json["env"]["CLAUDE_CONFIG_DIR"], dir.as_str());
    assert_eq!(json["argv"], json!(["-p", "hi"]));
}

#[test]
fn subscription_launches_never_get_a_key() {
    let fx = Fixture::new();
    // A usable subscription account: the paid account is never picked,
    // also with --allow-paid.
    fx.cache("main", 10.0, 10.0);
    fx.cache("angel2", 95.0, 10.0);
    for args in [&["-p", "x"][..], &["--allow-paid", "-p", "x"], &["@angel2"]] {
        let json = launched(&mut fx.ccx(args));
        assert_ne!(account_of(&json), "paid", "{args:?}");
        assert!(json["env"].get("ANTHROPIC_API_KEY").is_none(), "{args:?}");
    }
    // A claude subcommand on the base account.
    let json = launched(&mut fx.ccx(&["mcp", "list"]));
    assert!(json["env"].get("ANTHROPIC_API_KEY").is_none());
}

#[test]
fn named_paid_account_needs_a_yes_or_allow_paid() {
    let fx = Fixture::new();
    let err = refused(&mut fx.ccx(&["@paid", "-p", "x"]));
    assert!(err.contains("paid API-key account"), "{err}");
    assert!(err.contains("--allow-paid"), "{err}");

    for args in [
        &["@paid", "--allow-paid", "-p", "x"][..],
        &["--allow-paid", "@paid", "-p", "x"],
    ] {
        let json = launched(&mut fx.ccx(args));
        assert_eq!(account_of(&json), "paid");
        assert_eq!(json["env"]["ANTHROPIC_API_KEY"], KEY);
        assert_eq!(json["argv"], json!(["-p", "x"]));
    }
}

#[test]
fn on_a_terminal_ccx_asks_before_the_paid_fallback() {
    let fx = Fixture::new();
    fx.all_limited();
    let text = on_tty(&fx.ccx(&[]), &["-p", "hi"], "y");
    assert!(
        text.contains("All subscription accounts are limited. Use paid account paid?"),
        "{text}"
    );
    assert!(text.contains("\"CCX_ACTIVE\":\"paid\""), "{text}");

    let text = on_tty(&fx.ccx(&[]), &["-p", "hi"], "n");
    assert!(text.contains("Use paid account paid?"), "{text}");
    assert!(text.contains("\"CCX_ACTIVE\":\"main\""), "{text}");
    assert!(!text.contains("ANTHROPIC_API_KEY"), "{text}");
}

// Key sources.

#[test]
fn a_key_file_with_a_different_mode_is_refused() {
    let fx = Fixture::new();
    fx.write_key_file(KEY, 0o644);
    let err = refused(&mut fx.ccx(&["--allow-paid", "@paid"]));
    assert!(err.contains("mode 0644"), "{err}");
    assert!(err.contains("chmod 600"), "{err}");
    assert!(!err.contains(KEY), "{err}");
}

#[test]
fn key_command_gives_the_key_or_stops_the_launch() {
    let fx = Fixture::with_config(&config(&format!("key_command = \"printf '{KEY}\\\\n'\"")));
    let json = launched(&mut fx.ccx(&["--allow-paid", "@paid"]));
    assert_eq!(json["env"]["ANTHROPIC_API_KEY"], KEY);

    for (command, want) in [("exit 3", "failed"), ("true", "gave no key")] {
        fs::write(
            fx.config_path(),
            config(&format!("key_command = \"{command}\"")),
        )
        .unwrap();
        let err = refused(&mut fx.ccx(&["--allow-paid", "@paid"]));
        assert!(err.contains("key_command of account paid"), "{err}");
        assert!(err.contains(want), "{err}");
        assert!(err.contains("ccx did not start claude"), "{err}");
    }
}

#[test]
fn keyring_key_reaches_only_the_paid_exec() {
    let fx = Fixture::with_config(&config("key_store = \"keyring\""));
    fs::create_dir_all(fx.secret_item().parent().unwrap()).unwrap();
    fs::write(fx.secret_item(), KEY).unwrap();
    let json = launched(&mut fx.ccx(&["--allow-paid", "@paid"]));
    assert_eq!(json["env"]["ANTHROPIC_API_KEY"], KEY);

    fs::remove_file(fx.secret_item()).unwrap();
    let err = refused(&mut fx.ccx(&["--allow-paid", "@paid"]));
    assert!(err.contains("item `ccx-paid`"), "{err}");
}

// The key never leaks.

#[test]
fn the_key_is_not_in_the_config_cache_banner_or_dry_run() {
    let fx = Fixture::new();
    fx.all_limited();

    let out = fx
        .ccx(&["--allow-paid", "-p", "hi"])
        .env("CCX_DRY_RUN", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("account: paid"), "{text}");
    // The shell had a key too, so the line shows a change (`~`).
    assert!(text.contains("~ ANTHROPIC_API_KEY=<hidden>"), "{text}");
    assert!(!text.contains(SHELL_KEY), "{text}");
    assert!(!text.contains(KEY), "{text}");
    assert!(!stderr(&out).contains(KEY));

    // The banner on a terminal.
    let text = on_tty(&fx.ccx(&[]), &["--allow-paid"], "");
    assert!(text.contains("ccx → paid · api-key (paid)"), "{text}");
    let banner = text.lines().find(|l| l.contains("ccx →")).unwrap();
    assert!(!banner.contains(KEY), "{banner}");

    // A launch and the usage refresh write no key into a file.
    launched(&mut fx.ccx(&["--allow-paid", "-p", "hi"]));
    let status = fx.ccx(&["status", "--json"]).output().unwrap();
    assert!(!stdout(&status).contains(KEY));
    let all = fx.all_text();
    assert!(!all.contains(KEY));
    assert!(!fs::read_to_string(fx.config_path()).unwrap().contains(KEY));
}

// manage add, rm, status and list.

#[test]
fn add_api_key_stores_the_key_as_ccx_name_and_no_secret_in_the_config() {
    let fx = Fixture::with_config("[accounts.main]\ndir = \"~/.claude\"\n");
    let out = with_input(
        &mut fx.ccx(&["manage", "add", "paid", "--api-key", "-y"]),
        &format!("{KEY}\n"),
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("`ccx-paid`"), "{}", stdout(&out));
    assert!(!stdout(&out).contains(KEY));
    assert_eq!(fs::read_to_string(fx.secret_item()).unwrap(), KEY);
    let config = fs::read_to_string(fx.config_path()).unwrap();
    assert!(config.contains("[accounts.paid]"), "{config}");
    assert!(config.contains("kind = \"api_key\""), "{config}");
    assert!(config.contains("key_store = \"keyring\""), "{config}");
    assert!(!config.contains(KEY), "{config}");
    assert!(!fx.key_file().exists());
    // The dir is made and synced, and there is no login.
    assert!(fx.home().join(".claude-paid/skills/tdd").exists());
    assert!(!fx.log().exists(), "no claude call");

    let json = launched(&mut fx.ccx(&["--allow-paid", "@paid"]));
    assert_eq!(json["env"]["ANTHROPIC_API_KEY"], KEY);
}

#[test]
fn add_api_key_with_a_failing_store_offers_key_command_or_the_file() {
    let fx = Fixture::with_config("[accounts.main]\ndir = \"~/.claude\"\n");
    let before = fs::read_to_string(fx.config_path()).unwrap();
    let out = with_input(
        fx.ccx(&["manage", "add", "paid", "--api-key", "-y"])
            .env("FAKE_SECRET_FAIL", "1"),
        &format!("{KEY}\n"),
    );
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(err.contains("OS secret store is not available"), "{err}");
    assert!(err.contains("--key-command"), "{err}");
    assert!(err.contains("--key-file"), "{err}");
    assert!(!err.contains("panicked"), "{err}");
    assert!(!fx.key_file().exists(), "no file without a confirm");
    assert_eq!(fs::read_to_string(fx.config_path()).unwrap(), before);

    // `--key-file` is the confirm for the file.
    let out = with_input(
        fx.ccx(&["manage", "add", "paid", "--api-key", "--key-file", "-y"])
            .env("FAKE_SECRET_FAIL", "1"),
        &format!("{KEY}\n"),
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode(&fx.key_file()), 0o600);
    assert_eq!(mode(fx.key_file().parent().unwrap()), 0o700);
    let config = fs::read_to_string(fx.config_path()).unwrap();
    assert!(config.contains("key_store = \"file\""), "{config}");
    assert!(!config.contains(KEY), "{config}");
    let json = launched(&mut fx.ccx(&["--allow-paid", "@paid"]));
    assert_eq!(json["env"]["ANTHROPIC_API_KEY"], KEY);
}

#[test]
fn add_api_key_with_key_command_checks_the_command_and_keeps_no_copy() {
    let fx = Fixture::with_config("[accounts.main]\ndir = \"~/.claude\"\n");
    let before = fs::read_to_string(fx.config_path()).unwrap();
    let out = fx
        .ccx(&[
            "manage",
            "add",
            "paid",
            "--api-key",
            "--key-command",
            "true",
            "-y",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("gave no key"), "{}", stderr(&out));
    assert_eq!(fs::read_to_string(fx.config_path()).unwrap(), before);

    let command = format!("printf {KEY}");
    let out = fx
        .ccx(&[
            "manage",
            "add",
            "paid",
            "--api-key",
            "--key-command",
            &command,
            "-y",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let config = fs::read_to_string(fx.config_path()).unwrap();
    assert!(config.contains("key_command = "), "{config}");
    assert!(!fx.secret_item().exists());
    assert!(!fx.key_file().exists());
}

#[test]
fn rm_purge_deletes_the_stored_key() {
    let fx = Fixture::with_config(&config("key_store = \"keyring\""));
    fs::create_dir_all(fx.secret_item().parent().unwrap()).unwrap();
    fs::write(fx.secret_item(), KEY).unwrap();
    let out = fx
        .ccx(&["manage", "rm", "paid", "--purge", "-y"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!fx.secret_item().exists());
    assert!(
        !fx.home().join(".claude-paid").exists(),
        "moved to the trash"
    );
    assert!(
        !fs::read_to_string(fx.config_path())
            .unwrap()
            .contains("paid")
    );
    assert!(!fx.log().exists(), "no claude auth logout");

    let fx = Fixture::new();
    let out = fx
        .ccx(&["manage", "rm", "paid", "--purge", "-y"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!fx.key_file().exists());

    // Without --purge, the key stays and ccx shows how to delete it.
    let fx = Fixture::new();
    let out = fx.ccx(&["manage", "rm", "paid", "-y"]).output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(fx.key_file().exists());
    assert!(
        stdout(&out).contains("To delete it, run: rm"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn status_and_list_show_the_paid_kind_with_no_usage() {
    let fx = Fixture::new();
    fx.cache("main", 10.0, 10.0);
    fx.cache("angel2", 10.0, 10.0);
    let out = fx.ccx(&["status"]).output().unwrap();
    let text = stdout(&out);
    let row = text.lines().find(|l| l.starts_with("paid")).unwrap();
    assert!(row.contains("api-key (paid)"), "{text}");
    assert!(!row.contains("unknown"), "{row}");
    // No claude call for the paid account.
    let log = fs::read_to_string(fx.log()).unwrap_or_default();
    assert!(!log.contains(".claude-paid"), "{log}");

    let text = stdout(&fx.ccx(&["manage", "list"]).output().unwrap());
    let row = text.lines().find(|l| l.starts_with("paid")).unwrap();
    assert!(row.ends_with("api-key (paid)"), "{text}");

    let err = refused(&mut fx.ccx(&["usage", "refresh", "paid"]));
    assert!(err.contains("API-key account"), "{err}");
    let err = refused(&mut fx.ccx(&["manage", "login", "paid"]));
    assert!(err.contains("no login"), "{err}");
}

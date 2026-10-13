//! End-to-end tests for the copy of the user MCP servers and the
//! preference keys (#17): `ccx manage sync --prefs` and the last step of
//! `ccx manage add`. Each test uses a temp HOME and the fake claude in
//! `testdata/fakebin`, which logs each call to `FAKE_CLAUDE_LOG`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

const CCX: &str = env!("CARGO_BIN_EXE_ccx");

const CONFIG: &str = "\
[launch]
banner = false

[accounts.main]
dir = \"~/.claude\"

[accounts.angel2]
dir = \"~/.claude-angel2\"

[accounts.work]
dir = \"~/.claude-work\"
";

/// A placeholder value that must never be in the output.
const SECRET: &str = "placeholder-token-value";

fn base_json() -> Value {
    json!({
        "theme": "dark",
        "verbose": true,
        "autoUpdates": false,
        "hasCompletedOnboarding": true,
        "lastOnboardingVersion": "2.1.0",
        "fooPref": "base",
        "oauthAccount": {"organizationUuid": "base-org"},
        "userID": "base-user",
        "numStartups": 42,
        "cachedUsageUtilization": {"x": 1},
        "projects": {"/base": {}},
        "mcpServers": {
            "local": {"type": "stdio", "command": "local-mcp", "args": ["--x"], "env": {"TOKEN": SECRET}},
            "remote": {"type": "http", "url": "https://mcp.example.com/mcp"},
            "keyed": {"type": "http", "url": "https://k.example.com", "headers": {"Authorization": SECRET}},
            "shared": {"type": "stdio", "command": "base-version"},
        },
    })
}

fn angel2_json() -> Value {
    json!({
        "theme": "light",
        "oauthAccount": {"organizationUuid": "angel2-org", "emailAddress": "angel2@example.com"},
        "userID": "angel2-user",
        "numStartups": 3,
        "projects": {"/work": {"allowedTools": ["Bash"]}},
        "editorMode": "vim",
        "mcpServers": {"shared": {"type": "stdio", "command": "angel2-version"}},
    })
}

struct Fixture {
    home: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().expect("temp HOME");
        let fx = Fixture { home };
        fs::create_dir_all(fx.home().join(".claude")).unwrap();
        fs::create_dir_all(fx.dir("angel2")).unwrap();
        fs::create_dir_all(fx.dir("work")).unwrap();
        fs::create_dir_all(fx.home().join(".config/ccx")).unwrap();
        fx.write_config(CONFIG);
        fx.write_json(&fx.home().join(".claude.json"), &base_json());
        fx.write_json(&fx.claude_json("angel2"), &angel2_json());
        fx
    }

    fn home(&self) -> &Path {
        self.home.path()
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.home().join(format!(".claude-{name}"))
    }

    fn claude_json(&self, name: &str) -> PathBuf {
        self.dir(name).join(".claude.json")
    }

    fn write_config(&self, text: &str) {
        fs::write(self.home().join(".config/ccx/config.toml"), text).unwrap();
    }

    fn write_json(&self, path: &Path, value: &Value) {
        fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
    }

    fn read_json(&self, path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    fn log(&self) -> PathBuf {
        self.home().join("fake-claude.log")
    }

    /// The marker file that the `security` shim writes when ccx runs it.
    fn security_marker(&self) -> PathBuf {
        self.home().join("security-called")
    }

    fn ccx(&self, args: &[&str]) -> Command {
        let fakebin = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fakebin");
        // A `security` shim before all other dirs in PATH. It records each
        // call, so that a test finds a keychain access.
        let shim = self.home().join("shim");
        fs::create_dir_all(&shim).unwrap();
        let security = shim.join("security");
        fs::write(
            &security,
            format!(
                "#!/bin/sh\ntouch '{}'\nexit 1\n",
                self.security_marker().display()
            ),
        )
        .unwrap();
        fs::set_permissions(&security, fs::Permissions::from_mode(0o755)).unwrap();
        let mut cmd = Command::new(CCX);
        cmd.args(args)
            .current_dir(self.home())
            .env_clear()
            .env("HOME", self.home())
            .env(
                "PATH",
                format!("{}:{}:/usr/bin:/bin", shim.display(), fakebin.display()),
            )
            .env("FAKE_CLAUDE_LOG", self.log());
        cmd
    }

    fn calls(&self) -> Vec<Value> {
        fs::read_to_string(self.log())
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn mcp_calls(&self) -> Vec<(Vec<String>, Option<String>)> {
        self.calls()
            .into_iter()
            .filter(|c| c["argv"][0] == "mcp")
            .map(|c| {
                let argv = c["argv"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| a.as_str().unwrap().to_string())
                    .collect();
                let dir = c["env"]["CLAUDE_CONFIG_DIR"].as_str().map(str::to_string);
                (argv, dir)
            })
            .collect()
    }

    /// Writes `sessions/<pid>.json` into the account dir.
    fn session(&self, name: &str, file: &str, value: &Value) {
        let dir = self.dir(name).join("sessions");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(file), value.to_string()).unwrap();
    }
}

fn run(mut cmd: Command) -> Output {
    cmd.output().expect("run ccx")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn ok(cmd: Command) -> String {
    let out = run(cmd);
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        text(&out.stdout),
        text(&out.stderr)
    );
    assert!(!text(&out.stderr).contains(SECRET));
    text(&out.stdout)
}

fn fails(cmd: Command) -> String {
    let out = run(cmd);
    assert!(
        !out.status.success(),
        "stdout: {}\nstderr: {}",
        text(&out.stdout),
        text(&out.stderr)
    );
    text(&out.stdout) + &text(&out.stderr)
}

fn add_call(name: &str, config: &Value) -> Vec<String> {
    ["mcp", "add-json", "-s", "user", name]
        .iter()
        .map(|s| s.to_string())
        .chain([config.to_string()])
        .collect()
}

#[test]
fn sync_prefs_copies_servers_and_allowlisted_keys() {
    let fx = Fixture::new();
    let out = ok(fx.ccx(&["manage", "sync", "--prefs", "angel2"]));
    assert!(!out.contains(SECRET), "{out}");
    for line in [
        "  ADD    mcp keyed",
        "  ADD    mcp local",
        "  ADD    mcp remote",
        "  SKIP   mcp shared (the account has it)",
        "  SET    theme",
        "  SET    verbose",
        "MCP servers that need re-auth: remote",
        "run `/mcp` in `ccx @angel2`",
    ] {
        assert!(out.contains(line), "{line}\n{out}");
    }

    // The exact `mcp add-json` calls, each with the account dir.
    let base = base_json();
    let dir = fx.dir("angel2").display().to_string();
    let want: Vec<(Vec<String>, Option<String>)> = ["keyed", "local", "remote"]
        .iter()
        .map(|name| {
            (
                add_call(name, &base["mcpServers"][*name]),
                Some(dir.clone()),
            )
        })
        .collect();
    assert_eq!(fx.mcp_calls(), want);
    assert_eq!(fx.calls().len(), 3, "only the mcp add-json calls");

    // Golden: only allowlisted keys change. The servers come from claude.
    let mut want = angel2_json();
    for key in [
        "theme",
        "verbose",
        "autoUpdates",
        "hasCompletedOnboarding",
        "lastOnboardingVersion",
    ] {
        want[key] = base[key].clone();
    }
    for name in ["keyed", "local", "remote"] {
        want["mcpServers"][name] = base["mcpServers"][name].clone();
    }
    assert_eq!(fx.read_json(&fx.claude_json("angel2")), want);
    assert!(!fx.security_marker().exists());

    // A second run changes nothing and runs no claude.
    let out = ok(fx.ccx(&["manage", "sync", "--prefs", "angel2"]));
    assert!(!out.contains("ADD"), "{out}");
    assert!(!out.contains("SET"), "{out}");
    assert!(out.contains("  OK     theme"), "{out}");
    assert_eq!(fx.calls().len(), 3);
}

#[test]
fn dry_run_shows_the_plan_and_changes_nothing() {
    let fx = Fixture::new();
    let before = fs::read(fx.claude_json("angel2")).unwrap();
    let out = ok(fx.ccx(&["manage", "sync", "--prefs", "angel2", "--dry-run"]));
    assert!(out.contains("  ADD    mcp local"), "{out}");
    assert!(out.contains("  SET    theme"), "{out}");
    assert!(out.contains("Dry run. Nothing changed."), "{out}");
    assert_eq!(fs::read(fx.claude_json("angel2")).unwrap(), before);
    assert!(!fx.claude_json("work").exists());
    assert!(fx.calls().is_empty());
}

#[test]
fn all_copies_into_each_account_but_never_the_base() {
    let fx = Fixture::new();
    let base_before = fs::read(fx.home().join(".claude.json")).unwrap();
    ok(fx.ccx(&["manage", "sync", "--prefs", "--all"]));
    let dirs: Vec<Option<String>> = fx.mcp_calls().into_iter().map(|(_, d)| d).collect();
    let angel2 = Some(fx.dir("angel2").display().to_string());
    let work = Some(fx.dir("work").display().to_string());
    assert_eq!(dirs.iter().filter(|d| **d == angel2).count(), 3);
    assert_eq!(dirs.iter().filter(|d| **d == work).count(), 4);
    assert!(dirs.iter().all(|d| d.is_some()), "no call for the base");

    // A new `.claude.json` gets the servers and only the allowlisted keys.
    let got = fx.read_json(&fx.claude_json("work"));
    let mut keys: Vec<&str> = got
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "autoUpdates",
            "hasCompletedOnboarding",
            "lastOnboardingVersion",
            "mcpServers",
            "theme",
            "verbose"
        ]
    );
    assert_eq!(
        fs::read(fx.home().join(".claude.json")).unwrap(),
        base_before
    );
}

#[test]
fn config_allow_and_deny_change_the_allowlist() {
    let fx = Fixture::new();
    fx.write_config(&format!(
        "{CONFIG}\n[prefs]\nallow = [\"fooPref\", \"oauthAccount\", \"numStartups\"]\ndeny = [\"theme\"]\n"
    ));
    let out = run(fx.ccx(&["manage", "sync", "--prefs", "angel2"]));
    assert!(out.status.success(), "{}", text(&out.stderr));
    let stderr = text(&out.stderr);
    assert!(stderr.contains("never copies `oauthAccount`"), "{stderr}");
    assert!(stderr.contains("never copies `numStartups`"), "{stderr}");
    let got = fx.read_json(&fx.claude_json("angel2"));
    let before = angel2_json();
    assert_eq!(got["fooPref"], "base");
    assert_eq!(got["theme"], before["theme"]);
    assert_eq!(got["oauthAccount"], before["oauthAccount"]);
    assert_eq!(got["numStartups"], before["numStartups"]);
}

#[test]
fn a_running_session_blocks_the_write() {
    let fx = Fixture::new();
    let before = fs::read(fx.claude_json("angel2")).unwrap();
    // This test process is alive. With no `procStart`, the pid check alone
    // decides.
    let pid = std::process::id();
    fx.session(
        "angel2",
        &format!("{pid}.json"),
        &json!({"pid": pid, "sessionId": "s"}),
    );
    let out = fails(fx.ccx(&["manage", "sync", "--prefs", "angel2"]));
    assert!(
        out.contains(&format!("a session is running (pid {pid})")),
        "{out}"
    );
    assert_eq!(fs::read(fx.claude_json("angel2")).unwrap(), before);
    assert!(fx.mcp_calls().is_empty());
}

#[test]
fn a_stale_session_file_does_not_block_the_write() {
    let fx = Fixture::new();
    // A dead pid.
    let mut child = Command::new("true").spawn().unwrap();
    let dead = child.id();
    child.wait().unwrap();
    fx.session(
        "angel2",
        &format!("{dead}.json"),
        &json!({"pid": dead, "sessionId": "a"}),
    );
    // A live pid with a different start time: the pid is in use again.
    let pid = std::process::id();
    fx.session(
        "angel2",
        &format!("{pid}.json"),
        &json!({"pid": pid, "sessionId": "b", "procStart": "Mon Jan  1 00:00:00 2001"}),
    );
    ok(fx.ccx(&["manage", "sync", "--prefs", "angel2"]));
    assert_eq!(fx.read_json(&fx.claude_json("angel2"))["theme"], "dark");
}

#[test]
fn key_files_in_sessions_are_never_read() {
    let fx = Fixture::new();
    // If ccx read this `.key` file as a session file, the live pid in it
    // would block the write.
    let pid = std::process::id();
    fx.session(
        "angel2",
        &format!("{pid}.0123abcd.key"),
        &json!({"pid": pid, "sessionId": "k"}),
    );
    fx.session(
        "angel2",
        &format!("{pid}.json.key"),
        &json!({"pid": pid, "sessionId": "k"}),
    );
    ok(fx.ccx(&["manage", "sync", "--prefs", "angel2"]));
    assert_eq!(fx.read_json(&fx.claude_json("angel2"))["theme"], "dark");
}

#[test]
fn a_failed_server_add_is_reported_and_the_rest_continues() {
    let fx = Fixture::new();
    let mut cmd = fx.ccx(&["manage", "sync", "--prefs", "angel2"]);
    cmd.env("FAKE_MCP_FAIL", "local");
    let out = fails(cmd);
    assert!(out.contains("cannot add MCP server local"), "{out}");
    let got = fx.read_json(&fx.claude_json("angel2"));
    assert!(got["mcpServers"]["remote"].is_object());
    assert!(got["mcpServers"].get("local").is_none());
    assert_eq!(got["theme"], "dark");
}

#[test]
fn a_missing_base_claude_json_copies_nothing() {
    let fx = Fixture::new();
    fs::remove_file(fx.home().join(".claude.json")).unwrap();
    let before = fs::read(fx.claude_json("angel2")).unwrap();
    let out = ok(fx.ccx(&["manage", "sync", "--prefs", "--all"]));
    assert!(out.contains("does not exist. Nothing to copy."), "{out}");
    assert_eq!(fs::read(fx.claude_json("angel2")).unwrap(), before);
    assert!(fx.calls().is_empty());
}

#[test]
fn prefs_refuses_the_base_account_and_force() {
    let fx = Fixture::new();
    let out = fails(fx.ccx(&["manage", "sync", "--prefs", "main"]));
    assert!(out.contains("base account"), "{out}");
    let out = fails(fx.ccx(&["manage", "sync", "--prefs", "--force", "angel2"]));
    assert!(out.contains("--force"), "{out}");
}

#[test]
fn add_copies_servers_and_prefs_after_the_login() {
    let fx = Fixture::new();
    let out = ok(fx.ccx(&["manage", "add", "new", "-y"]));
    assert!(
        out.contains("MCP servers that need re-auth: remote"),
        "{out}"
    );
    let calls = fx.calls();
    let login = calls
        .iter()
        .position(|c| c["argv"][1] == "login")
        .expect("a login call");
    let first_mcp = calls
        .iter()
        .position(|c| c["argv"][0] == "mcp")
        .expect("an mcp call");
    assert!(login < first_mcp, "the copy comes after the login");
    let dir = Some(fx.dir("new").display().to_string());
    let base = base_json();
    let want: Vec<(Vec<String>, Option<String>)> = ["keyed", "local", "remote", "shared"]
        .iter()
        .map(|name| (add_call(name, &base["mcpServers"][*name]), dir.clone()))
        .collect();
    assert_eq!(fx.mcp_calls(), want);
    let got = fx.read_json(&fx.claude_json("new"));
    // The login wrote `oauthAccount`. It stays.
    assert_eq!(
        got["oauthAccount"]["organizationRateLimitTier"],
        "default_claude_max_5x"
    );
    assert_eq!(got["theme"], "dark");
    assert_eq!(got["hasCompletedOnboarding"], true);
    assert!(got.get("userID").is_none());
    assert!(!fx.security_marker().exists());
}

#[test]
fn add_with_no_prefs_or_no_login_copies_nothing() {
    let fx = Fixture::new();
    ok(fx.ccx(&["manage", "add", "new", "--no-prefs", "-y"]));
    assert!(fx.mcp_calls().is_empty());
    assert!(fx.read_json(&fx.claude_json("new")).get("theme").is_none());

    ok(fx.ccx(&["manage", "add", "other", "--no-login", "-y"]));
    assert!(fx.mcp_calls().is_empty());
    assert!(!fx.claude_json("other").exists());
}

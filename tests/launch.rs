//! End-to-end tests for `ccx [@name] [args...]`. Each test uses a temp
//! HOME and `testdata/fakebin/claude`, which prints its argv and env as JSON.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const CCX: &str = env!("CARGO_BIN_EXE_ccx");

const CONFIG: &str = r#"
[accounts.main]
dir = "~/.claude"

[accounts.angel2]
dir = "~/.claude-angel2"
"#;

const OVERLAY: &str = r#"
[accounts.main]
email = "main@example.com"

[accounts.angel2]
email = "work@example.com"
"#;

const STRIPPED: [&str; 4] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_SECURESTORAGE_CONFIG_DIR",
];

struct Fixture {
    home: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self::with_config(CONFIG)
    }

    fn with_config(config: &str) -> Self {
        let home = tempfile::tempdir().expect("temp HOME");
        let dir = home.path().join(".config/ccx");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), config).unwrap();
        std::fs::write(dir.join("config.local.toml"), OVERLAY).unwrap();
        Fixture { home }
    }

    fn home(&self) -> &Path {
        self.home.path()
    }

    fn fakebin() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fakebin")
    }

    /// The PATH: `extra` dirs first, then the fake claude, then the system
    /// dirs that `perl` and `sh` need.
    fn path(extra: &[&Path]) -> String {
        let mut dirs: Vec<String> = extra.iter().map(|d| d.display().to_string()).collect();
        dirs.push(Self::fakebin().display().to_string());
        dirs.push("/usr/bin:/bin".into());
        dirs.join(":")
    }

    /// A ccx command with a clean environment.
    fn ccx(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(CCX);
        cmd.args(args)
            .env_clear()
            .env("HOME", self.home())
            .env("PATH", Self::path(&[]));
        cmd
    }

    fn write_exe(&self, name: &str, text: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = self.home().join(name);
        std::fs::write(&path, text).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Runs the command and parses the JSON that the fake claude prints.
fn launched(mut cmd: impl std::borrow::BorrowMut<Command>) -> Value {
    let out = cmd.borrow_mut().output().expect("run ccx");
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    serde_json::from_slice(&out.stdout).expect("fake claude JSON")
}

fn argv(json: &Value) -> Vec<&str> {
    json["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect()
}

/// Runs ccx on a pseudo-terminal with `script`, so that stdout and stderr
/// are a TTY. Returns all output (stdout and stderr together).
fn on_tty(cmd: &mut Command, args: &[&str]) -> String {
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
fn named_account_gets_its_config_dir_and_argv() {
    let fx = Fixture::new();
    let json = launched(fx.ccx(&["@angel2", "--model", "opus"]));
    let want_dir = format!("{}/.claude-angel2", fx.home().display());
    assert_eq!(json["env"]["CLAUDE_CONFIG_DIR"], want_dir.as_str());
    assert_eq!(json["env"]["CCX_ACTIVE"], "angel2");
    assert_eq!(json["env"]["CCX_DEPTH"], "1");
    assert_eq!(argv(&json), ["--model", "opus"]);
    assert!(
        json["program"]
            .as_str()
            .unwrap()
            .starts_with(Fixture::fakebin().to_str().unwrap())
    );
}

#[test]
fn base_account_unsets_inherited_config_dir() {
    let fx = Fixture::new();
    let json = launched(
        fx.ccx(&["@main"])
            .env("CLAUDE_CONFIG_DIR", "/somewhere/else"),
    );
    assert!(json["env"].get("CLAUDE_CONFIG_DIR").is_none());
    assert_eq!(json["env"]["CCX_ACTIVE"], "main");
    assert!(argv(&json).is_empty());
}

#[test]
fn ccx_active_is_never_read() {
    let fx = Fixture::new();
    let angel2_dir = format!("{}/.claude-angel2", fx.home().display());
    for args in [&["@main"][..], &[]] {
        let json = launched(
            fx.ccx(args)
                .env("CCX_ACTIVE", "angel2")
                .env("CLAUDE_CONFIG_DIR", &angel2_dir),
        );
        assert_eq!(json["env"]["CCX_ACTIVE"], "main", "args {args:?}");
        assert!(json["env"].get("CLAUDE_CONFIG_DIR").is_none());
    }
}

#[test]
fn ccx_account_forces_the_account() {
    let fx = Fixture::new();
    let json = launched(fx.ccx(&[]).env("CCX_ACCOUNT", "angel2"));
    assert_eq!(json["env"]["CCX_ACTIVE"], "angel2");
    let json = launched(fx.ccx(&["-p", "hi"]).env("CCX_ACCOUNT", "angel2"));
    assert_eq!(argv(&json), ["-p", "hi"]);
    // `@name` wins over CCX_ACCOUNT.
    let json = launched(fx.ccx(&["@main"]).env("CCX_ACCOUNT", "angel2"));
    assert_eq!(json["env"]["CCX_ACTIVE"], "main");
}

#[test]
fn double_dash_passes_all_arguments() {
    let fx = Fixture::new();
    let json = launched(fx.ccx(&["--", "status"]));
    assert_eq!(argv(&json), ["status"]);
    assert_eq!(json["env"]["CCX_ACTIVE"], "main");
    let json = launched(fx.ccx(&["--", "@angel2", "--version"]));
    assert_eq!(argv(&json), ["@angel2", "--version"]);
}

#[test]
fn reserved_word_is_not_passed_to_claude() {
    let fx = Fixture::new();
    let out = fx.ccx(&["status"]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("`ccx status` is not ready yet"));
    assert!(out.stdout.is_empty());
}

#[test]
fn stripped_variables_are_removed_without_warnings_in_a_pipe() {
    let fx = Fixture::new();
    let mut cmd = fx.ccx(&["@angel2"]);
    for key in STRIPPED {
        cmd.env(key, "x");
    }
    let out = cmd.output().unwrap();
    assert!(out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    for key in STRIPPED {
        assert!(json["env"].get(key).is_none(), "{key} is still set");
    }
    // No banner and no warnings when the output is not a TTY.
    assert_eq!(stderr(&out), "");
}

#[test]
fn stripped_variables_warn_once_each_on_a_tty() {
    let fx = Fixture::new();
    let mut cmd = fx.ccx(&[]);
    for key in STRIPPED {
        cmd.env(key, "x");
    }
    let output = on_tty(&mut cmd, &["@angel2"]);
    for key in STRIPPED {
        let warning = format!("ccx: warning: removed {key};");
        assert_eq!(output.matches(&warning).count(), 1, "{key}: {output}");
    }
    assert_eq!(
        output.matches("ccx: account angel2 (").count(),
        1,
        "{output}"
    );
    let json_line = output.lines().find(|l| l.contains("\"argv\"")).unwrap();
    let json: Value = serde_json::from_str(&json_line[json_line.find('{').unwrap()..]).unwrap();
    for key in STRIPPED {
        assert!(json["env"].get(key).is_none(), "{key} is still set");
    }
}

#[test]
fn banner_can_be_turned_off() {
    let fx = Fixture::with_config(&format!("[launch]\nbanner = false\n{CONFIG}"));
    let output = on_tty(&mut fx.ccx(&[]), &["@angel2"]);
    assert!(!output.contains("ccx: account"), "{output}");
    assert!(output.contains("\"argv\""), "{output}");
}

#[test]
fn path_link_to_ccx_is_skipped() {
    let fx = Fixture::new();
    let link_dir = fx.home().join("linkbin");
    std::fs::create_dir(&link_dir).unwrap();
    std::os::unix::fs::symlink(CCX, link_dir.join("claude")).unwrap();
    let mut cmd = fx.ccx(&["@angel2"]);
    cmd.env("PATH", Fixture::path(&[&link_dir]));
    let json = launched(&mut cmd);
    assert_eq!(json["env"]["CCX_DEPTH"], "1");
    assert!(
        json["program"]
            .as_str()
            .unwrap()
            .starts_with(Fixture::fakebin().to_str().unwrap())
    );
}

#[test]
fn claude_that_runs_ccx_stops_at_the_depth_limit() {
    let fx = Fixture::new();
    let wrapper = fx.write_exe(
        "claude-wrapper",
        &format!("#!/bin/sh\nexec '{CCX}' \"$@\"\n"),
    );
    let out = fx
        .ccx(&["@angel2"])
        .env("CCX_CLAUDE_BIN", &wrapper)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("CCX_DEPTH is 5: ccx started itself 5 times"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn unknown_account_lists_known_names() {
    let fx = Fixture::new();
    let out = fx.ccx(&["@unknown"]).output().unwrap();
    assert!(!out.status.success());
    assert_eq!(
        stderr(&out),
        "ccx: unknown account `unknown`; known accounts: angel2, main\n"
    );
    let out = fx.ccx(&[]).env("CCX_ACCOUNT", "nope").output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("known accounts: angel2, main"));
}

#[test]
fn exec_failure_exits_127() {
    let fx = Fixture::new();
    let missing = fx.home().join("missing-claude");
    let out = fx
        .ccx(&["@angel2"])
        .env("CCX_CLAUDE_BIN", &missing)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(127));
    assert!(stderr(&out).contains("cannot run"), "{}", stderr(&out));

    let not_exe = fx.home().join("not-exe");
    std::fs::write(&not_exe, "text").unwrap();
    let config = format!("[launch]\nclaude_bin = \"~/not-exe\"\n{CONFIG}");
    std::fs::write(fx.home().join(".config/ccx/config.toml"), config).unwrap();
    let out = fx.ccx(&["@angel2"]).output().unwrap();
    assert_eq!(out.status.code(), Some(127));
}

#[test]
fn config_claude_bin_comes_before_ccx_claude_bin() {
    let fx = Fixture::with_config(&format!(
        "[launch]\nclaude_bin = \"{}/claude\"\n{CONFIG}",
        Fixture::fakebin().display()
    ));
    let json = launched(
        fx.ccx(&["@angel2"])
            .env("CCX_CLAUDE_BIN", "/missing/claude")
            .env("PATH", "/usr/bin:/bin"),
    );
    assert_eq!(json["env"]["CCX_ACTIVE"], "angel2");
}

#[test]
fn dry_run_prints_the_plan_and_does_not_run_claude() {
    let fx = Fixture::new();
    let out = fx
        .ccx(&["@angel2", "--model", "opus"])
        .env("CCX_DRY_RUN", "1")
        .env("ANTHROPIC_API_KEY", "secret-value")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let text = String::from_utf8(out.stdout).unwrap();
    let home = fx.home().display();
    let fakebin = Fixture::fakebin();
    let want = format!(
        "account: angel2
dir: {home}/.claude-angel2
claude: {}/claude
env:
  - ANTHROPIC_API_KEY
  + CCX_ACTIVE=angel2
  + CCX_DEPTH=1
  + CLAUDE_CONFIG_DIR={home}/.claude-angel2
argv: [\"--model\", \"opus\"]
",
        fakebin.display()
    );
    assert_eq!(text, want);
}

#[test]
fn missing_config_is_a_clear_error() {
    let fx = Fixture::new();
    std::fs::remove_file(fx.home().join(".config/ccx/config.toml")).unwrap();
    let out = fx.ccx(&["@angel2"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("cannot read config"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn ccx_config_and_xdg_config_home_are_used() {
    let fx = Fixture::new();
    let other = fx.home().join("repo/config.toml");
    std::fs::create_dir_all(other.parent().unwrap()).unwrap();
    std::fs::write(
        &other,
        "[accounts.main]\ndir = \"~/.claude\"\n[accounts.solo]\ndir = \"~/.claude-solo\"\n",
    )
    .unwrap();
    // The overlay in ~/.config/ccx names angel2, which this config lacks.
    let out = fx
        .ccx(&["@solo"])
        .env("CCX_CONFIG", &other)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("account `angel2` is not in"),
        "{}",
        stderr(&out)
    );

    // With XDG_CONFIG_HOME, the overlay comes from there (and there is none).
    let xdg = fx.home().join("xdg");
    std::fs::create_dir_all(&xdg).unwrap();
    let json = launched(
        fx.ccx(&["@solo"])
            .env("CCX_CONFIG", &other)
            .env("XDG_CONFIG_HOME", &xdg),
    );
    assert_eq!(json["env"]["CCX_ACTIVE"], "solo");
}

const BASE_NOTE: &str = "ccx: 'auth' runs on base account main; use @name for another account";

#[test]
fn subcommand_without_name_runs_on_the_base_account() {
    let fx = Fixture::new();
    let out = fx
        .ccx(&["auth", "status"])
        .env("CLAUDE_CONFIG_DIR", "/somewhere/else")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(json["env"].get("CLAUDE_CONFIG_DIR").is_none());
    assert_eq!(json["env"]["CCX_ACTIVE"], "main");
    assert_eq!(argv(&json), ["auth", "status"]);
    // No note when stderr is not a TTY.
    assert_eq!(stderr(&out), "");
}

#[test]
fn subcommand_with_name_runs_in_that_account() {
    let fx = Fixture::new();
    let json = launched(fx.ccx(&["@angel2", "mcp", "list"]));
    let want_dir = format!("{}/.claude-angel2", fx.home().display());
    assert_eq!(json["env"]["CLAUDE_CONFIG_DIR"], want_dir.as_str());
    assert_eq!(json["env"]["CCX_ACTIVE"], "angel2");
    assert_eq!(argv(&json), ["mcp", "list"]);
}

#[test]
fn ccx_account_selects_the_account_for_a_subcommand() {
    let fx = Fixture::new();
    let json = launched(fx.ccx(&["plugin", "list"]).env("CCX_ACCOUNT", "angel2"));
    let want_dir = format!("{}/.claude-angel2", fx.home().display());
    assert_eq!(json["env"]["CLAUDE_CONFIG_DIR"], want_dir.as_str());
    assert_eq!(json["env"]["CCX_ACTIVE"], "angel2");
    assert_eq!(argv(&json), ["plugin", "list"]);
}

#[test]
fn double_dash_subcommand_follows_the_same_rule() {
    let fx = Fixture::new();
    let json = launched(
        fx.ccx(&["--", "auth", "status"])
            .env("CLAUDE_CONFIG_DIR", "/somewhere/else"),
    );
    assert_eq!(argv(&json), ["auth", "status"]);
    assert!(json["env"].get("CLAUDE_CONFIG_DIR").is_none());
    assert_eq!(json["env"]["CCX_ACTIVE"], "main");

    let output = on_tty(&mut fx.ccx(&[]), &["--", "auth", "status"]);
    assert_eq!(output.matches(BASE_NOTE).count(), 1, "{output}");
}

#[test]
fn base_note_shows_only_on_a_tty_and_only_without_a_name() {
    let fx = Fixture::new();
    let output = on_tty(&mut fx.ccx(&[]), &["auth", "status"]);
    assert_eq!(output.matches(BASE_NOTE).count(), 1, "{output}");
    assert!(output.contains("\"argv\""), "{output}");

    for args in [
        &["@main", "auth", "status"][..],
        &["config"],
        &["-p", "auth"],
    ] {
        let output = on_tty(&mut fx.ccx(&[]), args);
        assert!(
            !output.contains("runs on base account"),
            "{args:?}: {output}"
        );
    }
    let output = on_tty(
        fx.ccx(&[]).env("CCX_ACCOUNT", "angel2"),
        &["auth", "status"],
    );
    assert!(!output.contains("runs on base account"), "{output}");
}

#[test]
fn config_is_not_a_subcommand() {
    let fx = Fixture::new();
    let out = fx.ccx(&["config"]).output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(argv(&json), ["config"]);
    assert_eq!(json["env"]["CCX_ACTIVE"], "main");
}

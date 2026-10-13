//! End-to-end tests for `ccx manage bootstrap` (#19). Each test uses a
//! temp HOME, a local git repo as `[base].repo` and the fakes in
//! `testdata/fakebin`: `claude`, `launchctl`, `systemctl` and `loginctl`.
//! `CCX_SERVICE_KIND=launchd` selects the fake launchd on all platforms. No
//! test touches a real account or loads a real job. All emails are
//! `@example.com` placeholders.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

const CCX: &str = env!("CARGO_BIN_EXE_ccx");
const PLIST: &str = "Library/LaunchAgents/io.github.thetechchild.ccx.refresh.plist";

/// The files in the base repo.
const REPO_FILES: [(&str, &str); 3] = [
    ("CLAUDE.md", "# Shared instructions\n"),
    ("settings.json", "{}\n"),
    ("agents/helper.md", "An agent.\n"),
];

struct Fixture {
    home: tempfile::TempDir,
    /// Not in HOME: the base repo, the fake service state and the logs.
    outside: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Fixture {
        Self::with_accounts(&["work", "loggedout1"])
    }

    fn with_accounts(names: &[&str]) -> Fixture {
        let fx = Fixture {
            home: tempfile::tempdir().expect("temp HOME"),
            outside: tempfile::tempdir().expect("temp dir"),
        };
        fs::create_dir_all(fx.config_dir()).unwrap();
        fs::create_dir(fx.outside.path().join("service")).unwrap();
        fx.make_repo();
        let mut config = format!(
            "[launch]\nbanner = false\n\n[base]\nrepo = \"{}\"\n\n[accounts.main]\ndir = \"~/.claude\"\n",
            fx.repo().display()
        );
        for name in names {
            config.push_str(&format!(
                "\n[accounts.{name}]\ndir = \"~/.claude-{name}\"\n"
            ));
        }
        fs::write(fx.config_dir().join("config.toml"), config).unwrap();
        // The overlay has the expected email of each account.
        let overlay: String = names
            .iter()
            .map(|name| format!("[accounts.{name}]\nemail = \"{name}@example.com\"\n\n"))
            .collect();
        fs::write(fx.overlay_path(), overlay).unwrap();
        fs::set_permissions(fx.overlay_path(), fs::Permissions::from_mode(0o600)).unwrap();
        fx
    }

    fn home(&self) -> PathBuf {
        fs::canonicalize(self.home.path()).unwrap()
    }

    fn base(&self) -> PathBuf {
        self.home().join(".claude")
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.home().join(format!(".claude-{name}"))
    }

    fn config_dir(&self) -> PathBuf {
        self.home.path().join(".config/ccx")
    }

    fn overlay_path(&self) -> PathBuf {
        self.config_dir().join("config.local.toml")
    }

    fn repo(&self) -> PathBuf {
        fs::canonicalize(self.outside.path())
            .unwrap()
            .join("base-repo")
    }

    fn log(&self) -> PathBuf {
        self.outside.path().join("claude.log")
    }

    /// A local git repo with `REPO_FILES` in one commit on branch `main`.
    fn make_repo(&self) {
        let repo = self.repo();
        fs::create_dir(&repo).unwrap();
        for (path, text) in REPO_FILES {
            let path = repo.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        for args in [
            &["-c", "init.defaultBranch=main", "init", "-q"][..],
            &["add", "-A"],
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "-m",
                "base",
            ],
        ] {
            let out = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .expect("run git");
            assert!(out.status.success(), "{}", text(&out.stderr));
        }
    }

    fn ccx(&self, args: &[&str]) -> Command {
        let fakebin = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fakebin");
        let mut cmd = Command::new(CCX);
        cmd.args(args)
            .current_dir(self.home())
            .env_clear()
            .env("HOME", self.home())
            .env("USER", "tester")
            .env("SHELL", "/bin/zsh")
            .env("PATH", format!("{}:/usr/bin:/bin", fakebin.display()))
            .env("CCX_SERVICE_KIND", "launchd")
            .env("FAKE_SERVICE_STATE", self.outside.path().join("service"))
            .env("FAKE_SERVICE_LOG", self.outside.path().join("service.log"))
            .env("FAKE_CLAUDE_LOG", self.log())
            .stdin(Stdio::null());
        cmd
    }

    /// The argv, `CLAUDE_CONFIG_DIR` and environment of each `auth login`
    /// call.
    fn logins(&self) -> Vec<Value> {
        fs::read_to_string(self.log())
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|c| c["argv"][0] == "auth" && c["argv"][1] == "login")
            .collect()
    }

    /// Each path in HOME with its kind and content, or its link target.
    /// The state dir of ccx (usage cache, probe dir) is not in the list.
    fn snapshot(&self) -> BTreeMap<PathBuf, String> {
        let mut out = BTreeMap::new();
        walk(&self.home(), &self.home(), &mut out);
        out
    }
}

fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let rel = path.strip_prefix(root).unwrap().to_path_buf();
        if rel.starts_with(".local/state") {
            continue;
        }
        let meta = fs::symlink_metadata(&path).unwrap();
        let mode = meta.permissions().mode() & 0o7777;
        if meta.file_type().is_symlink() {
            let target = fs::read_link(&path).unwrap();
            out.insert(rel, format!("link {}", target.display()));
        } else if meta.is_dir() {
            out.insert(rel, format!("dir {mode:o}"));
            walk(root, &path, out);
        } else {
            let bytes = fs::read(&path).unwrap();
            out.insert(rel, format!("file {mode:o} {}", text(&bytes)));
        }
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn run(mut cmd: Command) -> Output {
    cmd.output().expect("run ccx")
}

/// Runs a command that must pass. Returns stdout and stderr together.
fn ok(cmd: Command) -> String {
    let out = run(cmd);
    let all = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(out.status.success(), "{all}");
    all
}

/// Runs a command that must fail. Returns stdout and stderr together.
fn fails(cmd: Command) -> String {
    let out = run(cmd);
    let all = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(!out.status.success(), "{all}");
    all
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// The summary lines at the end of the output.
fn summary(out: &str) -> Vec<String> {
    let start = out.find("Bootstrap summary:\n").expect("a summary") + 19;
    out[start..]
        .lines()
        .take_while(|line| line.starts_with("  "))
        .map(|line| {
            line.split_whitespace()
                .take(2)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

#[test]
fn full_bootstrap_then_a_second_run_changes_nothing() {
    let fx = Fixture::new();
    let out = ok(fx.ccx(&["manage", "bootstrap", "--service"]));
    assert_eq!(
        summary(&out),
        [
            "OK claude",
            "DONE base",
            "DONE sync",
            "DONE login",
            "DONE service"
        ],
        "{out}"
    );

    // The base account is a clone of the repo.
    for (path, want) in REPO_FILES {
        assert_eq!(fs::read_to_string(fx.base().join(path)).unwrap(), want);
    }
    assert!(fx.base().join(".git").is_dir());

    // Sync made the account dirs at mode 0700 and the links.
    for name in ["work", "loggedout1"] {
        let dir = fx.dir(name);
        assert_eq!(mode(&dir), 0o700, "{name}");
        for item in ["CLAUDE.md", "settings.json", "agents"] {
            assert_eq!(
                fs::read_link(dir.join(item)).unwrap(),
                fx.base().join(item),
                "{name}/{item}"
            );
        }
    }

    // Only the account that is not logged in gets a login, with the email
    // from config.local.toml.
    let logins = fx.logins();
    assert_eq!(logins.len(), 1, "{logins:?}");
    assert_eq!(
        logins[0]["argv"],
        json!(["auth", "login", "--email", "loggedout1@example.com"])
    );
    assert_eq!(
        logins[0]["env"]["CLAUDE_CONFIG_DIR"],
        json!(fx.dir("loggedout1").to_str().unwrap())
    );
    assert!(
        out.contains("loggedout1: logged in as loggedout1@example.com"),
        "{out}"
    );

    // The service and the last step.
    assert!(fx.home().join(PLIST).is_file());
    assert!(out.contains("[6/6] status"), "{out}");
    assert!(
        out.contains("eval \"$(ccx manage shell-init zsh)\""),
        "{out}"
    );

    // A second run changes nothing and reports each step as OK.
    let before = fx.snapshot();
    let out = ok(fx.ccx(&["manage", "bootstrap", "--service"]));
    assert_eq!(
        summary(&out),
        ["OK claude", "OK base", "OK sync", "OK login", "OK service"],
        "{out}"
    );
    assert_eq!(fx.snapshot(), before);
    assert_eq!(fx.logins().len(), 1);
}

#[test]
fn missing_claude_shows_the_installer_and_fails() {
    let fx = Fixture::new();
    let mut cmd = fx.ccx(&["manage", "bootstrap"]);
    cmd.env("CCX_CLAUDE_BIN", fx.home().join("no-such-claude"));
    let out = fails(cmd);
    assert!(
        out.contains("curl -fsSL https://claude.ai/install.sh | bash"),
        "{out}"
    );
    assert!(!fx.base().exists());
}

#[test]
fn existing_base_dir_becomes_a_clone_and_keeps_its_files() {
    let fx = Fixture::new();
    fs::create_dir(fx.base()).unwrap();
    fs::write(fx.base().join("history.jsonl"), "my history\n").unwrap();
    fs::create_dir(fx.base().join("agents")).unwrap();
    fs::write(fx.base().join("agents/mine.md"), "my agent\n").unwrap();
    let out = ok(fx.ccx(&["manage", "bootstrap", "--no-service"]));
    assert!(out.contains("DONE  base"), "{out}");
    assert!(out.contains("is now a clone of"), "{out}");
    assert_eq!(
        fs::read_to_string(fx.base().join("history.jsonl")).unwrap(),
        "my history\n"
    );
    assert_eq!(
        fs::read_to_string(fx.base().join("agents/mine.md")).unwrap(),
        "my agent\n"
    );
    assert_eq!(
        fs::read_to_string(fx.base().join("CLAUDE.md")).unwrap(),
        REPO_FILES[0].1
    );
    // The local branch tracks the remote branch.
    let out = Command::new("git")
        .arg("-C")
        .arg(fx.base())
        .args(["rev-parse", "--abbrev-ref", "@{upstream}"])
        .env("HOME", fx.home())
        .output()
        .unwrap();
    assert_eq!(text(&out.stdout).trim(), "origin/main");
}

#[test]
fn existing_base_dir_with_a_repo_file_stops_and_changes_nothing() {
    let fx = Fixture::new();
    fs::create_dir(fx.base()).unwrap();
    fs::write(fx.base().join("CLAUDE.md"), "my own\n").unwrap();
    fs::write(fx.base().join("agents"), "a file\n").unwrap();
    let before = fx.snapshot();
    let out = fails(fx.ccx(&["manage", "bootstrap", "--no-service"]));
    assert!(out.contains("ccx does not overwrite files"), "{out}");
    assert!(out.contains("  CLAUDE.md\n"), "{out}");
    assert!(out.contains("  agents\n"), "{out}");
    assert!(!out.contains("[3/6]"), "{out}");
    assert_eq!(fx.snapshot(), before);
    assert!(fx.logins().is_empty());
}

#[test]
fn no_browser_login_shows_the_url_and_passes_the_code_on_stdin() {
    let fx = Fixture::with_accounts(&["loggedout1"]);
    let mut cmd = fx.ccx(&["manage", "bootstrap", "--no-service"]);
    cmd.env("SSH_CONNECTION", "192.0.2.1 50000 192.0.2.2 22")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("run ccx");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"fake-code-123\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let all = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(out.status.success(), "{all}");

    assert!(all.contains("ccx: no browser is available"), "{all}");
    assert!(
        all.contains("https://claude.example.com/oauth/authorize?code=true&state=fake-state"),
        "{all}"
    );
    assert_eq!(
        fs::read_to_string(fx.dir("loggedout1").join(".fake-login-code")).unwrap(),
        "fake-code-123"
    );
    // ccx adds no flag to `claude auth login`.
    let logins = fx.logins();
    assert_eq!(logins.len(), 1, "{logins:?}");
    assert_eq!(
        logins[0]["argv"],
        json!(["auth", "login", "--email", "loggedout1@example.com"])
    );
    assert!(all.contains("DONE  login"), "{all}");
}

#[test]
fn missing_overlay_warns_and_logs_in_without_the_email_check() {
    let fx = Fixture::with_accounts(&["loggedout1"]);
    fs::remove_file(fx.overlay_path()).unwrap();
    let out = ok(fx.ccx(&["manage", "bootstrap", "--no-service"]));
    assert!(
        out.contains("config.local.toml does not exist, so ccx does not check the email"),
        "{out}"
    );
    let logins = fx.logins();
    assert_eq!(logins.len(), 1, "{logins:?}");
    assert_eq!(logins[0]["argv"], json!(["auth", "login"]));
    assert!(
        out.contains("loggedout1: logged in (email not checked)"),
        "{out}"
    );
}

#[test]
fn service_is_skipped_with_no_terminal_or_with_no_service() {
    for args in [
        &["manage", "bootstrap"][..],
        &["manage", "bootstrap", "--no-service"],
    ] {
        let fx = Fixture::new();
        let out = ok(fx.ccx(args));
        assert!(summary(&out).contains(&"SKIP service".to_string()), "{out}");
        assert!(!fx.home().join(PLIST).exists());
    }
    let fx = Fixture::new();
    let out = ok(fx.ccx(&["manage", "bootstrap"]));
    assert!(
        out.contains("no terminal; to install, give --service"),
        "{out}"
    );
}

#[test]
fn a_failed_login_fails_bootstrap_after_all_steps() {
    let fx = Fixture::new();
    let mut cmd = fx.ccx(&["manage", "bootstrap", "--no-service"]);
    cmd.env("FAKE_LOGIN_FAIL", "1");
    let out = fails(cmd);
    assert!(summary(&out).contains(&"FAIL login".to_string()), "{out}");
    assert!(out.contains("[6/6] status"), "{out}");
}

//! End-to-end tests for `ccx manage add`. Each test uses a temp HOME and
//! the fake claude in `testdata/fakebin`, which fakes `auth login` and
//! `auth status` with `@example.com` emails and logs each call to
//! `FAKE_CLAUDE_LOG`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const CCX: &str = env!("CARGO_BIN_EXE_ccx");

const CONFIG: &str = "\
# My accounts.
[launch]
banner = false # quiet

[accounts.main]
dir = \"~/.claude\"

[accounts.angel2]
dir = \"~/.claude-angel2\"
";

const OVERLAY: &str = "\
# Private.
[accounts.main]
email = \"main@example.com\"
";

struct Fixture {
    home: tempfile::TempDir,
}

impl Fixture {
    /// HOME with a base account that has shared items, `angel2`, the
    /// config and the overlay.
    fn new() -> Self {
        let home = tempfile::tempdir().expect("temp HOME");
        let base = home.path().join(".claude");
        fs::create_dir_all(base.join("skills/tdd")).unwrap();
        fs::create_dir(base.join("agents")).unwrap();
        fs::write(base.join("settings.json"), "{}\n").unwrap();
        fs::write(base.join("CLAUDE.md"), "# synthetic\n").unwrap();
        fs::create_dir(home.path().join(".claude-angel2")).unwrap();
        let fx = Fixture { home };
        fs::create_dir_all(fx.config_dir()).unwrap();
        fs::write(fx.config_path(), CONFIG).unwrap();
        fs::write(fx.overlay_path(), OVERLAY).unwrap();
        fs::set_permissions(fx.overlay_path(), fs::Permissions::from_mode(0o600)).unwrap();
        fx
    }

    fn home(&self) -> &Path {
        self.home.path()
    }

    fn config_dir(&self) -> PathBuf {
        self.home().join(".config/ccx")
    }

    fn config_path(&self) -> PathBuf {
        self.config_dir().join("config.toml")
    }

    fn overlay_path(&self) -> PathBuf {
        self.config_dir().join("config.local.toml")
    }

    fn config(&self) -> String {
        fs::read_to_string(self.config_path()).unwrap()
    }

    fn overlay(&self) -> String {
        fs::read_to_string(self.overlay_path()).unwrap()
    }

    fn log(&self) -> PathBuf {
        self.home().join("fake-claude.log")
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

    /// The calls that the fake claude logged.
    fn calls(&self) -> Vec<Value> {
        fs::read_to_string(self.log())
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn login_calls(&self) -> Vec<Value> {
        self.calls()
            .into_iter()
            .filter(|c| c["argv"][1] == "login")
            .collect()
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
    text(&out.stderr)
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn add_makes_the_dir_syncs_logs_in_and_writes_the_config() {
    let fx = Fixture::new();
    let mut cmd = fx.ccx(&["manage", "add", "work", "--email", "work@example.com", "-y"]);
    // ccx must remove this variable for the login.
    cmd.env("ANTHROPIC_API_KEY", "synthetic-key");
    let stdout = ok(cmd);
    assert!(stdout.contains("ccx manage add will:"), "{stdout}");
    assert!(
        stdout.contains("work: kind max, tier Max 5x, weight 5"),
        "{stdout}"
    );

    // The dir has mode 0700 and the sync links.
    let dir = fx.home().join(".claude-work");
    assert_eq!(mode(&dir), 0o700);
    let base = fx.home().join(".claude");
    for item in ["settings.json", "CLAUDE.md", "agents", "skills/tdd"] {
        assert_eq!(
            fs::read_link(dir.join(item)).unwrap(),
            base.join(item),
            "{item}"
        );
    }
    assert!(
        !fs::symlink_metadata(dir.join("skills"))
            .unwrap()
            .is_symlink()
    );

    // One login, in the new dir, with the email.
    let logins = fx.login_calls();
    assert_eq!(logins.len(), 1);
    assert_eq!(
        logins[0]["argv"],
        serde_json::json!(["auth", "login", "--email", "work@example.com"])
    );
    assert_eq!(logins[0]["env"]["CLAUDE_CONFIG_DIR"], dir.to_str().unwrap());
    assert!(logins[0]["env"].get("ANTHROPIC_API_KEY").is_none());

    // config.toml keeps its comments and entries and gets no email.
    let config = fx.config();
    assert_eq!(
        config,
        format!("{CONFIG}\n[accounts.work]\ndir = \"~/.claude-work\"\n")
    );
    assert!(!config.contains("email"), "{config}");
    assert!(!config.contains("@"), "{config}");

    // The email goes only to the overlay, at mode 0600.
    assert_eq!(
        fx.overlay(),
        format!("{OVERLAY}\n[accounts.work]\nemail = \"work@example.com\"\n")
    );
    assert_eq!(mode(&fx.overlay_path()), 0o600);

    // The new account loads with the normal loader.
    let list = ok(fx.ccx(&["manage", "list", "--json"]));
    assert!(list.contains("\"work\""), "{list}");
    assert!(list.contains("work@example.com"), "{list}");
}

#[test]
fn add_with_no_email_uses_the_email_of_the_login() {
    let fx = Fixture::new();
    let stdout = ok(fx.ccx(&["manage", "add", "work", "-y"]));
    assert_eq!(
        fx.login_calls()[0]["argv"],
        serde_json::json!(["auth", "login"])
    );
    assert!(fx.overlay().contains("email = \"work@example.com\""));
    assert!(!fx.config().contains("email"));
    assert!(stdout.contains("tier Max 5x"), "{stdout}");
}

#[test]
fn an_unknown_tier_shows_the_raw_string() {
    let fx = Fixture::new();
    let mut cmd = fx.ccx(&["manage", "add", "work", "-y"]);
    cmd.env("FAKE_LOGIN_TIER", "synthetic_new_tier")
        .env("FAKE_LOGIN_ORG_TYPE", "claude_team");
    let stdout = ok(cmd);
    assert!(
        stdout.contains("work: kind max, tier synthetic_new_tier, weight 5"),
        "{stdout}"
    );

    let fx = Fixture::new();
    let mut cmd = fx.ccx(&["manage", "add", "big", "-y"]);
    cmd.env("FAKE_LOGIN_TIER", "default_claude_max_20x");
    let stdout = ok(cmd);
    assert!(stdout.contains("tier Max 20x, weight 20"), "{stdout}");
}

#[test]
fn an_email_mismatch_warns_and_writes_no_config_without_a_terminal() {
    let fx = Fixture::new();
    let mut cmd = fx.ccx(&["manage", "add", "work", "--email", "work@example.com", "-y"]);
    cmd.env("FAKE_LOGIN_EMAIL", "other@example.com");
    let stderr = fails(cmd);
    assert!(
        stderr.contains("the login is other@example.com, but --email is work@example.com"),
        "{stderr}"
    );
    assert!(stderr.contains("The config did not change"), "{stderr}");
    assert_eq!(fx.config(), CONFIG);
    assert_eq!(fx.overlay(), OVERLAY);

    // Add again: the dir from the first run is accepted.
    ok(fx.ccx(&["manage", "add", "work", "--email", "work@example.com", "-y"]));
    assert!(fx.overlay().contains("email = \"work@example.com\""));
}

#[test]
fn add_without_a_terminal_and_without_yes_changes_nothing() {
    let fx = Fixture::new();
    let stderr = fails(fx.ccx(&["manage", "add", "work"]));
    assert!(stderr.contains("no terminal"), "{stderr}");
    assert!(stderr.contains("ccx manage add work -y"), "{stderr}");
    assert!(!fx.home().join(".claude-work").exists());
    assert!(fx.calls().is_empty());
    assert_eq!(fx.config(), CONFIG);
}

#[test]
fn add_without_a_name_and_without_a_terminal_names_the_flags() {
    let fx = Fixture::new();
    // No terminal: no wizard starts, also with `-y`.
    for args in [
        &["manage", "add"][..],
        &["manage", "add", "-y", "--no-login"],
    ] {
        let stderr = fails(fx.ccx(args));
        assert!(stderr.contains("missing the account name NAME"), "{stderr}");
        assert!(stderr.contains("no terminal for the wizard"), "{stderr}");
        assert!(
            stderr.contains("`ccx manage add NAME [--dir DIR] [--email EMAIL] [--no-login] [--no-prefs] [--api-key] -y`"),
            "{stderr}"
        );
    }
    assert!(fx.calls().is_empty());
    assert_eq!(fx.config(), CONFIG);
}

#[test]
fn add_prints_a_summary_with_the_email_and_the_tier() {
    let fx = Fixture::new();
    let stdout = ok(fx.ccx(&["manage", "add", "work", "--email", "work@example.com", "-y"]));
    assert!(
        stdout.contains("Summary: account work, dir ~/.claude-work, email work@example.com"),
        "{stdout}"
    );
    assert!(stdout.contains("work: kind "), "{stdout}");
}

#[test]
fn no_login_stops_after_sync() {
    let fx = Fixture::new();
    let stdout = ok(fx.ccx(&["manage", "add", "work", "--no-login", "-y"]));
    assert!(fx.calls().is_empty(), "{:?}", fx.calls());
    let dir = fx.home().join(".claude-work");
    assert_eq!(mode(&dir), 0o700);
    assert!(dir.join("settings.json").is_symlink());
    assert!(fx.config().contains("[accounts.work]"));
    assert_eq!(fx.overlay(), OVERLAY);
    assert!(stdout.contains("ccx @work auth login"), "{stdout}");
}

#[test]
fn add_uses_the_dir_option() {
    let fx = Fixture::new();
    ok(fx.ccx(&[
        "manage",
        "add",
        "work",
        "--dir",
        "~/accounts/work",
        "--no-login",
        "-y",
    ]));
    let dir = fx.home().join("accounts/work");
    assert_eq!(mode(&dir), 0o700);
    assert!(dir.join("CLAUDE.md").is_symlink());
    assert!(fx.config().contains("dir = \"~/accounts/work\""));
}

#[test]
fn add_refuses_bad_names_and_duplicates() {
    let fx = Fixture::new();
    for (args, want) in [
        (vec!["Bad"], "not a valid account name"),
        (vec!["angel2"], "already exists"),
        (
            vec!["new", "--dir", "~/.claude-angel2"],
            "`angel2` already uses",
        ),
        (vec!["new", "--dir", "~/.claude"], "`main` already uses"),
    ] {
        let mut all = vec!["manage", "add"];
        all.extend(args);
        all.push("-y");
        let stderr = fails(fx.ccx(&all));
        assert!(stderr.contains(want), "{all:?}: {stderr}");
    }
    assert_eq!(fx.config(), CONFIG);
    assert!(fx.calls().is_empty());
}

#[test]
fn add_refuses_a_dir_with_other_files() {
    let fx = Fixture::new();
    let dir = fx.home().join(".claude-work");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("notes.txt"), "").unwrap();
    let stderr = fails(fx.ccx(&["manage", "add", "work", "-y"]));
    assert!(stderr.contains("ccx manage adopt"), "{stderr}");
    assert_eq!(fx.config(), CONFIG);
}

#[test]
fn a_failed_login_writes_no_config() {
    let fx = Fixture::new();
    let mut cmd = fx.ccx(&["manage", "add", "work", "-y"]);
    cmd.env("FAKE_LOGIN_FAIL", "1");
    let stderr = fails(cmd);
    assert!(stderr.contains("`claude auth login` failed"), "{stderr}");
    assert_eq!(fx.config(), CONFIG);
    assert_eq!(fx.overlay(), OVERLAY);
    // A second add works.
    ok(fx.ccx(&["manage", "add", "work", "-y"]));
    assert!(fx.config().contains("[accounts.work]"));
}

#[test]
fn add_with_no_overlay_makes_it_at_mode_0600() {
    let fx = Fixture::new();
    fs::remove_file(fx.overlay_path()).unwrap();
    ok(fx.ccx(&["manage", "add", "work", "-y"]));
    assert_eq!(
        fx.overlay(),
        "[accounts.work]\nemail = \"work@example.com\"\n"
    );
    assert_eq!(mode(&fx.overlay_path()), 0o600);
    assert!(!fx.config().contains("email"));
}

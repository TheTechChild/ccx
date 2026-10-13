//! End-to-end tests for `ccx manage adopt` and `ccx manage list`. Each test
//! uses a temp HOME with synthetic account dirs and the fake claude in
//! `testdata/fakebin`, which answers `auth status --json` with
//! `@example.com` emails and logs each call to `FAKE_CLAUDE_LOG`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

const CCX: &str = env!("CARGO_BIN_EXE_ccx");

struct Fixture {
    home: tempfile::TempDir,
}

impl Fixture {
    /// HOME with `~/.claude`, `~/.claude-angel2` and the stray
    /// `~/.claude/.claude.json` and `~/.claude/.claude.json.lock`.
    fn new() -> Self {
        let home = tempfile::tempdir().expect("temp HOME");
        let base = home.path().join(".claude");
        fs::create_dir(&base).unwrap();
        fs::create_dir(home.path().join(".claude-angel2")).unwrap();
        fs::write(base.join(".claude.json"), "{\"synthetic\": true}\n").unwrap();
        fs::create_dir(base.join(".claude.json.lock")).unwrap();
        // Not account dirs.
        fs::write(home.path().join(".claude.json"), "{}\n").unwrap();
        fs::create_dir(home.path().join(".config")).unwrap();
        Fixture { home }
    }

    fn home(&self) -> &Path {
        self.home.path()
    }

    fn config_dir(&self) -> PathBuf {
        self.home().join(".config/ccx")
    }

    fn log(&self) -> PathBuf {
        self.home().join("fake-claude.log")
    }

    fn ccx(&self, args: &[&str]) -> Command {
        let fakebin = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fakebin");
        let mut cmd = Command::new(CCX);
        cmd.args(args)
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

    /// The paths of the files in the backup dirs, relative to the root.
    fn backup_files(&self) -> Vec<String> {
        let root = self.home().join(".local/state/ccx/backup");
        let Ok(dirs) = fs::read_dir(&root) else {
            return Vec::new();
        };
        let mut files = Vec::new();
        for dir in dirs {
            let dir = dir.unwrap().path();
            for file in fs::read_dir(&dir).unwrap() {
                let path = file.unwrap().path();
                files.push(path.strip_prefix(&root).unwrap().display().to_string());
            }
        }
        files.sort();
        files
    }
}

fn run(mut cmd: Command) -> Output {
    cmd.output().expect("run ccx")
}

fn ok(cmd: Command) -> String {
    let out = run(cmd);
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

const CONFIG: &str = "\
[accounts.main]
dir = \"~/.claude\"

[accounts.angel2]
dir = \"~/.claude-angel2\"
";

const OVERLAY: &str = "\
[accounts.main]
email = \"main@example.com\"

[accounts.angel2]
email = \"angel2@example.com\"
";

#[test]
fn adopt_writes_both_files_and_backs_up_the_stray_files() {
    let fx = Fixture::new();
    let mut cmd = fx.ccx(&["manage", "adopt", "-y"]);
    // ccx must unset these for the base account and replace them for others.
    cmd.env("CLAUDE_CONFIG_DIR", "/somewhere/else")
        .env("ANTHROPIC_API_KEY", "synthetic-key");
    let stdout = ok(cmd);
    assert!(stdout.contains("add account main = ~/.claude"), "{stdout}");
    assert!(
        stdout.contains("add account angel2 = ~/.claude-angel2"),
        "{stdout}"
    );

    // config.toml: dirs only, no email key.
    let config = fs::read_to_string(fx.config_dir().join("config.toml")).unwrap();
    assert_eq!(config, CONFIG);
    assert!(!config.contains("email"));

    // config.local.toml: the emails, at mode 0600.
    let overlay_path = fx.config_dir().join("config.local.toml");
    assert_eq!(fs::read_to_string(&overlay_path).unwrap(), OVERLAY);
    let mode = fs::metadata(&overlay_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);

    // The stray files moved to the backup and nothing was deleted.
    let base = fx.home().join(".claude");
    assert!(!base.join(".claude.json").exists());
    assert!(!base.join(".claude.json.lock").exists());
    let files = fx.backup_files();
    assert_eq!(files.len(), 2, "{files:?}");
    assert!(files[0].ends_with("/.claude.json"), "{files:?}");
    assert!(files[1].ends_with("/.claude.json.lock"), "{files:?}");
    let backup = fx.home().join(".local/state/ccx/backup").join(&files[0]);
    assert_eq!(
        fs::read_to_string(backup).unwrap(),
        "{\"synthetic\": true}\n"
    );
    assert!(
        stdout.contains(".local/state/ccx/backup/adopt-"),
        "{stdout}"
    );
    // The real ~/.claude.json of the base account stays.
    assert!(fx.home().join(".claude.json").exists());

    // The env that the fake claude got.
    let calls = fx.calls();
    assert_eq!(calls.len(), 2);
    for call in &calls {
        assert_eq!(call["argv"], json!(["auth", "status", "--json"]));
        assert!(call["env"].get("ANTHROPIC_API_KEY").is_none());
    }
    let main = &calls[0]["env"];
    assert!(main.get("CLAUDE_CONFIG_DIR").is_none(), "{main}");
    assert_eq!(main["CCX_ACTIVE"], "main");
    let angel2 = &calls[1]["env"];
    let want = format!("{}/.claude-angel2", fx.home().display());
    assert_eq!(angel2["CLAUDE_CONFIG_DIR"], want.as_str());
    assert_eq!(angel2["CCX_ACTIVE"], "angel2");

    // The files load with the config loader: a launch works.
    let mut launch = fx.ccx(&["@angel2"]);
    launch.env("CCX_DRY_RUN", "1");
    let report = ok(launch);
    assert!(report.contains("angel2"), "{report}");

    // A second adopt has nothing to do.
    let stdout = ok(fx.ccx(&["manage", "adopt"]));
    assert!(stdout.contains("Nothing to adopt"), "{stdout}");
}

#[test]
fn adopt_without_a_terminal_or_yes_writes_nothing() {
    let fx = Fixture::new();
    let out = run(fx.ccx(&["manage", "adopt"]));
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no terminal"), "{stderr}");
    assert!(stderr.contains("ccx manage adopt -y"), "{stderr}");
    // The plan was shown.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("add account angel2"), "{stdout}");

    assert!(!fx.config_dir().exists());
    assert!(fx.home().join(".claude/.claude.json").exists());
    assert!(fx.home().join(".claude/.claude.json.lock").exists());
    assert!(fx.backup_files().is_empty());
}

#[test]
fn adopt_writes_through_a_config_symlink_and_keeps_other_entries() {
    let fx = Fixture::new();
    let repo = fx.home().join("dotfiles");
    fs::create_dir(&repo).unwrap();
    let existing = "\
# Shared ccx config.
[launch]
banner = false # No banner.

[accounts.home]
dir = \"~/.claude\"
aliases = [\"c1\"]
";
    fs::write(repo.join("config.toml"), existing).unwrap();
    fs::create_dir_all(fx.config_dir()).unwrap();
    let link = fx.config_dir().join("config.toml");
    std::os::unix::fs::symlink(repo.join("config.toml"), &link).unwrap();

    ok(fx.ccx(&["manage", "adopt", "--yes"]));

    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let config = fs::read_to_string(repo.join("config.toml")).unwrap();
    // The base account keeps its name `home`. Comments stay.
    assert_eq!(
        config,
        format!("{existing}\n[accounts.angel2]\ndir = \"~/.claude-angel2\"\n")
    );
    // The overlay is at the overlay path, never next to the link target.
    assert!(!repo.join("config.local.toml").exists());
    let overlay = fs::read_to_string(fx.config_dir().join("config.local.toml")).unwrap();
    assert!(overlay.contains("[accounts.home]\nemail = \"main@example.com\""));
    assert!(overlay.contains("[accounts.angel2]\nemail = \"angel2@example.com\""));
}

#[test]
fn adopt_uses_ccx_config_and_xdg_config_home() {
    let fx = Fixture::new();
    let config_path = fx.home().join("elsewhere/my-config.toml");
    let xdg = fx.home().join("xdg");
    let mut cmd = fx.ccx(&["manage", "adopt", "-y"]);
    cmd.env("CCX_CONFIG", &config_path)
        .env("XDG_CONFIG_HOME", &xdg);
    ok(cmd);
    assert_eq!(fs::read_to_string(&config_path).unwrap(), CONFIG);
    assert_eq!(
        fs::read_to_string(xdg.join("ccx/config.local.toml")).unwrap(),
        OVERLAY
    );
    assert!(!fx.home().join("elsewhere/config.local.toml").exists());
}

#[test]
fn adopt_without_the_base_dir_is_an_error() {
    let fx = Fixture::new();
    fs::remove_dir_all(fx.home().join(".claude")).unwrap();
    let out = run(fx.ccx(&["manage", "adopt", "-y"]));
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot find the base account"), "{stderr}");
    assert!(!fx.config_dir().exists());
}

#[test]
fn adopt_skips_emails_of_accounts_with_no_login() {
    let fx = Fixture::new();
    fs::create_dir(fx.home().join(".claude-loggedout")).unwrap();
    let stdout = ok(fx.ccx(&["manage", "adopt", "-y"]));
    assert!(stdout.contains("no email for loggedout"), "{stdout}");
    let config = fs::read_to_string(fx.config_dir().join("config.toml")).unwrap();
    assert!(config.contains("[accounts.loggedout]"));
    let overlay = fs::read_to_string(fx.config_dir().join("config.local.toml")).unwrap();
    assert!(!overlay.contains("loggedout"), "{overlay}");
}

#[test]
fn list_prints_a_table_and_stable_json() {
    let fx = Fixture::new();
    ok(fx.ccx(&["manage", "adopt", "-y"]));

    let table = ok(fx.ccx(&["manage", "list"]));
    assert_eq!(
        table,
        "\
NAME    DIR               EMAIL               ALIASES  KIND
angel2  ~/.claude-angel2  angel2@example.com  -        subscription
main    ~/.claude         main@example.com    -        subscription
"
    );

    let text = ok(fx.ccx(&["manage", "list", "--json"]));
    let home = fx.home().display().to_string();
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        value,
        json!({"accounts": [
            {"aliases": [], "base": false, "dir": format!("{home}/.claude-angel2"),
             "email": "angel2@example.com", "kind": "subscription", "name": "angel2"},
            {"aliases": [], "base": true, "dir": format!("{home}/.claude"),
             "email": "main@example.com", "kind": "subscription", "name": "main"},
        ]})
    );
    // Keys print in a fixed order.
    let name = text.find("\"name\"").unwrap();
    assert!(text.find("\"aliases\"").unwrap() < name);
    assert_eq!(text, ok(fx.ccx(&["manage", "list", "--json"])));
}

#[test]
fn list_without_a_config_is_an_error() {
    let fx = Fixture::new();
    let out = run(fx.ccx(&["manage", "list"]));
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read config"));
}

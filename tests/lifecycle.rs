//! End-to-end tests for `ccx manage login`, `logout`, `edit`,
//! `edit-config` and `rm`. Each test uses a temp HOME and the fake claude
//! in `testdata/fakebin`, which fakes `auth login`, `auth logout` and
//! `auth status` with `@example.com` emails and logs each argv and
//! environment to `FAKE_CLAUDE_LOG`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

const CCX: &str = env!("CARGO_BIN_EXE_ccx");

const CONFIG: &str = "\
# My accounts.
[launch]
banner = false # quiet

[accounts.main]
dir = \"~/.claude\"

[accounts.work]
dir = \"~/.claude-work\" # the work dir
weight = 2 # small

[accounts.loggedout1]
dir = \"~/.claude-loggedout1\"
";

const OVERLAY: &str = "\
# Private.
[accounts.work]
email = \"work@example.com\"
";

struct Fixture {
    home: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self::with(CONFIG, OVERLAY)
    }

    fn with(config: &str, overlay: &str) -> Self {
        let home = tempfile::tempdir().expect("temp HOME");
        for dir in [".claude", ".claude-work", ".claude-loggedout1"] {
            fs::create_dir(home.path().join(dir)).unwrap();
        }
        let fx = Fixture { home };
        fs::create_dir_all(fx.config_dir()).unwrap();
        fs::write(fx.config_path(), config).unwrap();
        fs::write(fx.overlay_path(), overlay).unwrap();
        fs::set_permissions(fx.overlay_path(), fs::Permissions::from_mode(0o600)).unwrap();
        fx
    }

    fn home(&self) -> &Path {
        self.home.path()
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.home().join(format!(".claude-{name}"))
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

    fn trash(&self) -> PathBuf {
        self.home().join(".local/state/ccx/trash")
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

    /// The argv and `CLAUDE_CONFIG_DIR` of each call other than `auth
    /// status`.
    fn changes(&self) -> Vec<(Value, Value)> {
        self.calls()
            .into_iter()
            .filter(|c| c["argv"] != json!(["auth", "status", "--json"]))
            .map(|c| (c["argv"].clone(), c["env"]["CLAUDE_CONFIG_DIR"].clone()))
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

/// Runs a command that must fail. Returns stdout and stderr together.
fn fails(cmd: Command) -> String {
    let out = run(cmd);
    let all = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(!out.status.success(), "{all}");
    all
}

fn dir_value(path: &Path) -> Value {
    json!(path.to_str().unwrap())
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

// login

#[test]
fn login_runs_in_the_account_dir_with_the_expected_email() {
    let fx = Fixture::new();
    let mut cmd = fx.ccx(&["manage", "login", "work"]);
    // ccx must remove this variable for the login.
    cmd.env("ANTHROPIC_API_KEY", "synthetic-key");
    let stdout = ok(cmd);
    assert!(
        stdout.contains("work: logged in as work@example.com"),
        "{stdout}"
    );
    assert_eq!(
        fx.changes(),
        [(
            json!(["auth", "login", "--email", "work@example.com"]),
            dir_value(&fx.dir("work"))
        )]
    );
    let login = &fx.calls()[0];
    assert!(login["env"].get("ANTHROPIC_API_KEY").is_none());
    assert_eq!(fx.config(), CONFIG);
    assert_eq!(fx.overlay(), OVERLAY);
}

#[test]
fn login_fails_when_the_email_is_not_the_expected_email() {
    let fx = Fixture::new();
    let mut cmd = fx.ccx(&["manage", "login", "work"]);
    cmd.env("FAKE_LOGIN_EMAIL", "other@example.com");
    let out = fails(cmd);
    assert!(
        out.contains(
            "the login of account work is other@example.com, but config.local.toml expects work@example.com"
        ),
        "{out}"
    );
    assert!(out.contains("ccx manage logout work"), "{out}");
    assert_eq!(
        fx.changes(),
        [(
            json!(["auth", "login", "--email", "work@example.com"]),
            dir_value(&fx.dir("work"))
        )]
    );
}

#[test]
fn login_with_no_expected_email_warns_and_skips_the_check() {
    let fx = Fixture::new();
    let out = run(fx.ccx(&["manage", "login", "loggedout1"]));
    assert!(out.status.success(), "{}", text(&out.stderr));
    let stderr = text(&out.stderr);
    assert!(
        stderr.contains("config.local.toml has no email for account loggedout1"),
        "{stderr}"
    );
    assert!(
        text(&out.stdout).contains("loggedout1: logged in (email not checked)"),
        "{}",
        text(&out.stdout)
    );
    assert_eq!(
        fx.changes(),
        [(json!(["auth", "login"]), dir_value(&fx.dir("loggedout1")))]
    );
}

#[test]
fn login_of_an_unknown_account_is_an_error() {
    let fx = Fixture::new();
    let out = fails(fx.ccx(&["manage", "login", "nobody"]));
    assert!(out.contains("no account `nobody`"), "{out}");
    assert!(fx.calls().is_empty());
}

#[test]
fn login_all_logs_in_only_the_logged_out_accounts_with_a_summary() {
    let config = format!("{CONFIG}\n[accounts.loggedout2]\ndir = \"~/.claude-loggedout2\"\n");
    // The fake login gives `loggedout1@example.com` to the two accounts,
    // so the check of `loggedout2` fails.
    let overlay = format!(
        "{OVERLAY}\n[accounts.loggedout1]\nemail = \"loggedout1@example.com\"\n\n[accounts.loggedout2]\nemail = \"x@example.com\"\n"
    );
    let fx = Fixture::with(&config, &overlay);
    fs::create_dir(fx.dir("loggedout2")).unwrap();
    let mut cmd = fx.ccx(&["manage", "login", "--all"]);
    cmd.env("FAKE_LOGIN_EMAIL", "loggedout1@example.com");
    let out = fails(cmd);
    for line in [
        "Summary:",
        "  loggedout1: logged in as loggedout1@example.com",
        "  loggedout2: FAILED: the login of account loggedout2 is loggedout1@example.com, but config.local.toml expects x@example.com",
        "  main: logged in already",
        "  work: logged in already",
    ] {
        assert!(out.contains(line), "{line}\n{out}");
    }
    assert_eq!(
        fx.changes(),
        [
            (
                json!(["auth", "login", "--email", "loggedout1@example.com"]),
                dir_value(&fx.dir("loggedout1"))
            ),
            (
                json!(["auth", "login", "--email", "x@example.com"]),
                dir_value(&fx.dir("loggedout2"))
            ),
        ]
    );
    // `auth status` for the base account has no CLAUDE_CONFIG_DIR.
    let base_status = fx
        .calls()
        .into_iter()
        .find(|c| c["env"].get("CCX_ACTIVE") == Some(&json!("main")))
        .unwrap();
    assert!(base_status["env"].get("CLAUDE_CONFIG_DIR").is_none());
}

#[test]
fn login_all_succeeds_when_all_accounts_are_logged_in() {
    let fx = Fixture::new();
    let mut cmd = fx.ccx(&["manage", "login", "--all"]);
    cmd.env("FAKE_LOGIN_EMAIL", "loggedout1@example.com");
    let stdout = ok(cmd);
    assert!(
        stdout.contains("loggedout1: logged in (email not checked)"),
        "{stdout}"
    );
    let stdout = ok(fx.ccx(&["manage", "login", "--all"]));
    assert!(stdout.contains("loggedout1: logged in already"), "{stdout}");
    assert_eq!(fx.changes().len(), 1);
}

// logout

#[test]
fn logout_runs_claude_auth_logout_for_that_account_only() {
    let fx = Fixture::new();
    let stdout = ok(fx.ccx(&["manage", "logout", "work", "-y"]));
    assert!(stdout.contains("Logged out of account work."), "{stdout}");
    assert_eq!(
        fx.changes(),
        [(json!(["auth", "logout"]), dir_value(&fx.dir("work")))]
    );
    assert!(fx.dir("work").join(".fake-logout").exists());
    assert!(!fx.home().join(".claude/.fake-logout").exists());
    assert!(!fx.home().join(".fake-logout").exists());
}

#[test]
fn logout_of_the_base_account_has_no_config_dir() {
    let fx = Fixture::new();
    ok(fx.ccx(&["manage", "logout", "main", "-y"]));
    assert_eq!(fx.changes(), [(json!(["auth", "logout"]), Value::Null)]);
}

#[test]
fn logout_without_a_terminal_and_without_yes_refuses() {
    let fx = Fixture::new();
    let out = fails(fx.ccx(&["manage", "logout", "work"]));
    assert!(out.contains("ccx manage logout work -y"), "{out}");
    assert!(fx.calls().is_empty());
}

// edit

#[test]
fn edit_changes_only_the_given_fields_and_keeps_comments() {
    let fx = Fixture::new();
    let stdout = ok(fx.ccx(&["manage", "edit", "work", "--weight", "5", "--disable"]));
    assert!(stdout.contains("work: weight = 5"), "{stdout}");
    assert!(stdout.contains("work: enabled = false"), "{stdout}");
    assert_eq!(
        fx.config(),
        CONFIG.replace(
            "weight = 2 # small\n",
            "weight = 5 # small\nenabled = false\n"
        )
    );
    assert_eq!(fx.overlay(), OVERLAY);

    ok(fx.ccx(&["manage", "edit", "work", "--enable", "--weight", "0.5"]));
    assert_eq!(
        fx.config(),
        CONFIG.replace(
            "weight = 2 # small\n",
            "weight = 0.5 # small\nenabled = true\n"
        )
    );
    assert!(fx.calls().is_empty());
}

#[test]
fn edit_email_writes_only_to_the_overlay() {
    let fx = Fixture::new();
    ok(fx.ccx(&["manage", "edit", "loggedout1", "--email", "lo@example.com"]));
    let config = fx.config();
    assert_eq!(config, CONFIG);
    assert!(!config.contains("email"), "{config}");
    assert!(!config.contains('@'), "{config}");
    assert_eq!(
        fx.overlay(),
        format!("{OVERLAY}\n[accounts.loggedout1]\nemail = \"lo@example.com\"\n")
    );
    assert_eq!(mode(&fx.overlay_path()), 0o600);

    // A new email replaces the old one.
    ok(fx.ccx(&["manage", "edit", "work", "--email", "new@example.com"]));
    assert!(fx.overlay().contains("email = \"new@example.com\""));
    assert!(!fx.overlay().contains("work@example.com"));
    assert!(!fx.config().contains('@'));
}

#[test]
fn edit_rejects_values_that_are_not_valid() {
    let fx = Fixture::new();
    for args in [
        &["work", "--weight", "0"][..],
        &["work", "--weight", "-1"],
        &["work", "--weight", "nan"],
        &["work", "--weight", "many"],
        &["work", "--email", "not-an-email"],
        &["work", "--enable", "--disable"],
        &["work"],
        &["nobody", "--weight", "1"],
    ] {
        let mut all = vec!["manage", "edit"];
        all.extend(args);
        fails(fx.ccx(&all));
    }
    assert_eq!(fx.config(), CONFIG);
    assert_eq!(fx.overlay(), OVERLAY);
}

// edit-config

#[test]
fn edit_config_writes_a_valid_edit() {
    let fx = Fixture::new();
    let new = fx.home().join("new.toml");
    let text = CONFIG.replace("banner = false # quiet", "banner = true # loud");
    fs::write(&new, &text).unwrap();
    let mut cmd = fx.ccx(&["manage", "edit-config"]);
    cmd.env("EDITOR", format!("cp {}", new.display()));
    ok(cmd);
    assert_eq!(fx.config(), text);
    assert!(!fx.home().join(".local/state/ccx/config.edit.toml").exists());
}

#[test]
fn edit_config_keeps_the_old_file_when_the_edit_is_not_valid() {
    let fx = Fixture::new();
    let new = fx.home().join("new.toml");
    fs::write(&new, format!("{CONFIG}\n[launch2]\n")).unwrap();
    let mut cmd = fx.ccx(&["manage", "edit-config"]);
    cmd.env("EDITOR", format!("cp {}", new.display()));
    let out = fails(cmd);
    assert!(out.contains("the edited config is not valid"), "{out}");
    assert!(out.contains("launch2"), "{out}");
    assert!(out.contains("did not change"), "{out}");
    assert_eq!(fx.config(), CONFIG);
    // The edit stays, so that the user can fix it.
    let copy = fx.home().join(".local/state/ccx/config.edit.toml");
    assert_eq!(
        fs::read_to_string(copy).unwrap(),
        format!("{CONFIG}\n[launch2]\n")
    );

    // A real email in config.toml is not valid (decision D2).
    let email = ["someone", "example.org"].join("@");
    fs::write(
        &new,
        CONFIG.replace(
            "weight = 2 # small",
            &format!("weight = 2\nemail = \"{email}\""),
        ),
    )
    .unwrap();
    let mut cmd = fx.ccx(&["manage", "edit-config"]);
    cmd.env("EDITOR", format!("cp {}", new.display()));
    let out = fails(cmd);
    assert!(out.contains("config.local.toml"), "{out}");
    assert_eq!(fx.config(), CONFIG);

    // A failed editor changes nothing.
    let mut cmd = fx.ccx(&["manage", "edit-config"]);
    cmd.env("EDITOR", "false");
    let out = fails(cmd);
    assert!(out.contains("the editor `false` failed"), "{out}");
    assert_eq!(fx.config(), CONFIG);
}

// rm

#[test]
fn rm_removes_the_entry_keeps_the_dir_and_prints_the_keychain_name() {
    let fx = Fixture::new();
    fs::write(fx.dir("work").join("history.jsonl"), "synthetic\n").unwrap();
    let stdout = ok(fx.ccx(&["manage", "rm", "work", "-y"]));
    assert!(stdout.contains("ccx manage rm will:"), "{stdout}");
    assert!(stdout.contains("Claude Code-credentials-"), "{stdout}");
    assert!(
        stdout.contains("security delete-generic-password -s"),
        "{stdout}"
    );
    let config = fx.config();
    assert!(!config.contains("work"), "{config}");
    assert!(config.starts_with("# My accounts.\n[launch]\nbanner = false # quiet\n"));
    assert!(config.contains("[accounts.loggedout1]"));
    assert!(!fx.overlay().contains("work"), "{}", fx.overlay());
    assert!(fx.dir("work").join("history.jsonl").exists());
    assert!(!fx.trash().exists());
    assert!(fx.calls().is_empty());
    // The config still loads.
    ok(fx.ccx(&["manage", "list"]));
}

#[test]
fn rm_purge_logs_out_deletes_the_credentials_file_and_moves_the_dir_to_trash() {
    let fx = Fixture::new();
    let dir = fx.dir("work");
    fs::write(dir.join(".credentials.json"), "synthetic\n").unwrap();
    fs::write(dir.join("history.jsonl"), "synthetic\n").unwrap();
    let stdout = ok(fx.ccx(&["manage", "rm", "work", "--purge", "-y"]));
    assert!(stdout.contains("Logged out of account work."), "{stdout}");
    assert_eq!(fx.changes(), [(json!(["auth", "logout"]), dir_value(&dir))]);
    assert!(!dir.exists());
    assert_eq!(mode(&fx.trash()), 0o700);
    let moved: Vec<PathBuf> = fs::read_dir(fx.trash())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(moved.len(), 1);
    let name = moved[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("work-"), "{name}");
    assert!(moved[0].join("history.jsonl").exists());
    assert!(moved[0].join(".fake-logout").exists());
    assert!(!moved[0].join(".credentials.json").exists());
    assert!(!fx.config().contains("work"));
    assert!(!fx.overlay().contains("work"));
}

#[test]
fn rm_purge_continues_when_the_logout_fails() {
    let fx = Fixture::new();
    let mut cmd = fx.ccx(&["manage", "rm", "loggedout1", "--purge", "-y"]);
    cmd.env("FAKE_LOGOUT_FAIL", "1");
    let out = run(cmd);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains("warning"),
        "{}",
        text(&out.stderr)
    );
    assert!(!fx.dir("loggedout1").exists());
    assert!(!fx.config().contains("loggedout1"));
}

#[test]
fn rm_refuses_the_base_account() {
    let fx = Fixture::new();
    for args in [
        &["manage", "rm", "main", "-y"][..],
        &["manage", "rm", "main", "--purge", "-y"],
    ] {
        let out = fails(fx.ccx(args));
        assert!(out.contains("is the base account"), "{out}");
    }
    assert!(fx.calls().is_empty());
    assert_eq!(fx.config(), CONFIG);
    assert!(fx.home().join(".claude").is_dir());
}

#[test]
fn rm_without_a_terminal_and_without_yes_refuses() {
    let fx = Fixture::new();
    let out = fails(fx.ccx(&["manage", "rm", "work", "--purge"]));
    assert!(out.contains("ccx manage rm work --purge -y"), "{out}");
    assert!(fx.calls().is_empty());
    assert!(fx.dir("work").is_dir());
    assert_eq!(fx.config(), CONFIG);
}

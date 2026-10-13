//! End-to-end tests for `ccx manage doctor`. Each test uses a temp HOME, a
//! synthetic account tree, the fake `claude` and the fake `security` in
//! `testdata/fakebin`. No test reads the real keychain.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sha2::{Digest, Sha256};

const CCX: &str = env!("CARGO_BIN_EXE_ccx");
const REPO: &str = env!("CARGO_MANIFEST_DIR");
const GOLDEN: &str = "testdata/doctor/problems.txt";

struct Fixture {
    home: tempfile::TempDir,
    keychain: Vec<String>,
}

impl Fixture {
    fn new(config: &str) -> Fixture {
        let fx = Fixture {
            home: tempfile::tempdir().expect("temp HOME"),
            keychain: Vec::new(),
        };
        write(&fx.home().join(".config/ccx/config.toml"), config);
        fs::create_dir(fx.base()).unwrap();
        fs::create_dir(fx.angel2()).unwrap();
        fx
    }

    fn home(&self) -> &Path {
        self.home.path()
    }

    fn base(&self) -> PathBuf {
        self.home().join(".claude")
    }

    fn angel2(&self) -> PathBuf {
        self.home().join(".claude-angel2")
    }

    fn ccx(&self, args: &[&str]) -> Command {
        let fakebin = Path::new(REPO).join("testdata/fakebin");
        let mut cmd = Command::new(CCX);
        cmd.args(args)
            .env_clear()
            .env("HOME", self.home())
            .env("PATH", format!("{}:/usr/bin:/bin", fakebin.display()))
            .env("FAKE_KEYCHAIN_SERVICES", self.keychain.join("\n"))
            .env("FAKE_CLAUDE_VERSION", "2.1.296 (Claude Code)");
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.ccx(args).output().expect("run ccx")
    }
}

const CONFIG: &str = "\
[accounts.main]
dir = \"~/.claude\"

[accounts.angel2]
dir = \"~/.claude-angel2\"
";

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

fn text(out: &Output) -> String {
    format!(
        "status: {}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The keychain service name of an account dir (as Claude Code makes it).
fn service_name(dir: &Path) -> String {
    let digest = Sha256::digest(dir.as_os_str().as_encoded_bytes());
    let hex: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
    format!("Claude Code-credentials-{hex}")
}

/// Each path under `root` (never through a symlink) with its kind, content
/// or link target, and mode.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path.strip_prefix(root).unwrap().to_path_buf();
            let meta = fs::symlink_metadata(&path).unwrap();
            let mtime = meta.modified().unwrap();
            if meta.file_type().is_symlink() {
                let target = fs::read_link(&path).unwrap();
                out.insert(rel, format!("link {}", target.display()));
            } else if meta.is_dir() {
                out.insert(rel, format!("dir {mtime:?}"));
                walk(root, &path, out);
            } else {
                let bytes = fs::read(&path).unwrap();
                out.insert(rel, format!("file {mtime:?} {bytes:?}"));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// A base account and an angel2 account that sync links completely.
fn clean(fx: &Fixture) {
    let base = fx.base();
    write(&base.join("settings.json"), "{\"hooks\":{}}");
    write(&base.join("CLAUDE.md"), "# rules");
    write(&base.join("skills/tdd/SKILL.md"), "tdd");
    fs::create_dir(base.join("projects")).unwrap();
    write(&base.join(".credentials.json"), "not read");
    let out = fx.run(&["manage", "sync", "--all"]);
    assert!(out.status.success(), "{}", text(&out));
}

/// One example of each problem.
fn problems(fx: &mut Fixture) {
    let (base, angel2, home) = (fx.base(), fx.angel2(), fx.home().display().to_string());
    clean(fx);
    // A `/config` write replaced the settings.json link with a real file.
    fs::remove_file(angel2.join("settings.json")).unwrap();
    write(&angel2.join("settings.json"), "{}");
    // A local-only skill. `synced` is never a local-only skill.
    write(&angel2.join("skills/my-local/SKILL.md"), "local");
    fs::create_dir(angel2.join("skills/synced")).unwrap();
    // An unknown base entry and the stray identity file.
    fs::create_dir(base.join("new-dir")).unwrap();
    write(&base.join(".claude.json"), "{}");
    // A hook and a status line with the absolute home path.
    let settings = format!(
        r#"{{
  "hooks": {{
    "Stop": [{{ "hooks": [{{ "type": "command", "command": "{home}/.claude/hooks/notify.sh" }}] }}]
  }},
  "statusLine": {{ "type": "command", "command": "python3 $HOME/.claude/statusline.py" }}
}}"#
    );
    write(&base.join("settings.json"), &settings);
    // An account dir with a trailing slash in the config.
    write(
        &fx.home().join(".config/ccx/config.toml"),
        &CONFIG.replace("~/.claude-angel2\"", "~/.claude-angel2/\""),
    );
    // A keychain item for angel2, the base item and one orphan.
    fx.keychain = vec![
        "Claude Code-credentials".into(),
        service_name(&angel2),
        "Claude Code-credentials-0badf00d".into(),
        "Other app".into(),
    ];
    // A linked skill that is not in the base is not a local-only skill.
    symlink(base.join("skills/tdd"), angel2.join("skills/tdd-copy")).unwrap();
}

/// The doctor text with the repo path as `<REPO>`. On Linux, the keychain
/// lines are not in the output.
fn normalize(text: &str) -> String {
    text.replace(REPO, "<REPO>")
}

fn golden() -> String {
    let golden = fs::read_to_string(Path::new(REPO).join(GOLDEN)).unwrap();
    if cfg!(target_os = "macos") {
        return golden;
    }
    // Remove the keychain lines and count the levels again.
    let lines: Vec<_> = golden
        .lines()
        .filter(|line| line.contains(": ") && !line.contains(" keychain: "))
        .collect();
    let count = |level: &str| lines.iter().filter(|l| l.starts_with(level)).count();
    format!(
        "{}\n\n{} OK, {} WARN, {} FAIL\n",
        lines.join("\n"),
        count("OK "),
        count("WARN "),
        count("FAIL ")
    )
}

#[test]
fn golden_text_for_one_example_of_each_problem() {
    let mut fx = Fixture::new(CONFIG);
    problems(&mut fx);
    let mut cmd = fx.ccx(&["manage", "doctor"]);
    cmd.env("CLAUDE_CODE_OAUTH_TOKEN", "secret-value");
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
    let got = normalize(&stdout(&out));
    assert!(!got.contains("secret-value"));
    assert!(
        !got.contains("@example.com"),
        "doctor shows no keychain account"
    );
    if std::env::var_os("CCX_UPDATE_GOLDEN").is_some() && cfg!(target_os = "macos") {
        let path = Path::new(REPO).join(GOLDEN);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, &got).unwrap();
    }
    assert_eq!(got, golden(), "\n{got}");
}

#[test]
fn doctor_changes_no_file() {
    let mut fx = Fixture::new(CONFIG);
    problems(&mut fx);
    let before = snapshot(fx.home());
    for args in [&["manage", "doctor"][..], &["manage", "doctor", "--json"]] {
        let out = fx.run(args);
        assert_eq!(out.status.code(), Some(2), "{}", text(&out));
    }
    assert_eq!(before, snapshot(fx.home()));
}

#[test]
fn clean_setup_exits_0() {
    let fx = Fixture::new(CONFIG);
    clean(&fx);
    let out = fx.run(&["manage", "doctor"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let got = stdout(&out);
    assert!(
        got.contains("OK    env: no override variables are set"),
        "{got}"
    );
    assert!(
        got.contains("OK    link: angel2: 4 shared items are correct"),
        "{got}"
    );
    assert!(
        got.lines()
            .all(|l| !l.starts_with("WARN") && !l.starts_with("FAIL")),
        "{got}"
    );
}

#[test]
fn override_var_only_exits_1() {
    let fx = Fixture::new(CONFIG);
    clean(&fx);
    for key in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "CLAUDE_SECURESTORAGE_CONFIG_DIR",
        "CLAUDE_CONFIG_DIR",
    ] {
        let mut cmd = fx.ccx(&["manage", "doctor"]);
        cmd.env(key, "x");
        let out = cmd.output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{}", text(&out));
        let got = stdout(&out);
        assert!(got.contains(&format!("WARN  env: {key} is set")), "{got}");
    }
}

#[test]
fn json_output_and_exit_codes() {
    let fx = Fixture::new(CONFIG);
    clean(&fx);
    let json = |out: &Output| -> serde_json::Value {
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", text(out)))
    };

    let out = fx.run(&["manage", "doctor", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let value = json(&out);
    assert_eq!(value["result"], "OK");
    let checks = value["checks"].as_array().unwrap();
    assert!(checks.iter().all(|c| c["level"] == "OK"));
    assert!(
        checks
            .iter()
            .any(|c| c["check"] == "link" && c["message"] == "angel2: 4 shared items are correct")
    );

    let mut cmd = fx.ccx(&["manage", "doctor", "--json"]);
    cmd.env("ANTHROPIC_API_KEY", "x");
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert_eq!(json(&out)["result"], "WARN");

    fs::remove_file(fx.angel2().join("CLAUDE.md")).unwrap();
    write(&fx.angel2().join("CLAUDE.md"), "changed");
    let out = fx.run(&["manage", "doctor", "--json"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
    let value = json(&out);
    assert_eq!(value["result"], "FAIL");
    assert!(value["checks"].as_array().unwrap().iter().any(|c| {
        c["level"] == "FAIL"
            && c["check"] == "link"
            && c["message"]
                .as_str()
                .unwrap()
                .starts_with("angel2: CLAUDE.md is a real file")
    }));
}

#[test]
fn bad_config_is_a_fail_and_other_checks_still_run() {
    let fx = Fixture::new("not toml [");
    let out = fx.run(&["manage", "doctor"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
    let got = stdout(&out);
    assert!(got.starts_with("FAIL  config: "), "{got}");
    assert!(got.contains("OK    claude: "), "{got}");
    assert!(got.contains("OK    env: "), "{got}");
}

#[test]
fn missing_claude_is_a_fail() {
    let fx = Fixture::new(CONFIG);
    clean(&fx);
    let mut cmd = fx.ccx(&["manage", "doctor"]);
    let dir = fx.home().join("bin");
    fs::create_dir(&dir).unwrap();
    cmd.env("PATH", &dir);
    let out = cmd.output().unwrap();
    let got = stdout(&out);
    // The fallback dirs can have a real claude on a developer machine.
    if got.contains("FAIL  claude: ") {
        assert_eq!(out.status.code(), Some(2), "{}", text(&out));
        assert!(got.contains("cannot find claude on PATH"), "{got}");
    } else {
        assert!(got.contains("claude is not on PATH"), "{got}");
    }
}

//! End-to-end tests for the automatic handoff on `ccx -c` and
//! `ccx --resume <id>` (#15). Each test uses a temp HOME with two account
//! directories, synthetic usage cache files and `testdata/fakebin/claude`,
//! which prints its argv, cwd and env as JSON. No test touches a real
//! account.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

const CCX: &str = env!("CARGO_BIN_EXE_ccx");

const CONFIG: &str = r#"
[accounts.main]
dir = "~/.claude"

[accounts.angel2]
dir = "~/.claude-angel2"
"#;

const ID: &str = "11111111-1111-4111-8111-111111111111";
const OTHER_ID: &str = "22222222-2222-4222-8222-222222222222";

/// A reset time far in the future, so that no window has reset.
const LATER: &str = "2099-01-01T00:00:00Z";

struct Fixture {
    _tmp: tempfile::TempDir,
    /// The physical path of the temp HOME.
    home: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("temp HOME");
        let home = fs::canonicalize(tmp.path()).unwrap();
        let dir = home.join(".config/ccx");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.toml"), CONFIG).unwrap();
        fs::create_dir_all(home.join("work/proj")).unwrap();
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::create_dir_all(home.join(".claude-angel2")).unwrap();
        Fixture { _tmp: tmp, home }
    }

    fn main(&self) -> PathBuf {
        self.home.join(".claude")
    }

    fn angel2(&self) -> PathBuf {
        self.home.join(".claude-angel2")
    }

    fn proj(&self) -> PathBuf {
        self.home.join("work/proj")
    }

    fn state(&self) -> PathBuf {
        self.home.join(".local/state/ccx/state.json")
    }

    fn ccx(&self, args: &[&str]) -> Command {
        let fakebin = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fakebin");
        let mut cmd = Command::new(CCX);
        cmd.args(args)
            .env_clear()
            .env("HOME", &self.home)
            .env("PATH", format!("{}:/usr/bin:/bin", fakebin.display()))
            .env("CCX_NO_REFRESH", "1")
            .current_dir(self.proj());
        cmd
    }

    /// Writes a usage cache file for `name` with a session window at
    /// `session` percent.
    fn usage(&self, name: &str, session: f64) {
        let path = self
            .home
            .join(format!(".local/state/ccx/usage/{name}.json"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let snapshot = json!({
            "source": "/usage",
            "fetched_at_ms": now_ms() - 60_000,
            "limits": [
                {"kind": "session", "percent": session, "resets_at": LATER, "scope": null},
                {"kind": "weekly_all", "percent": 10, "resets_at": LATER, "scope": null},
            ],
        });
        fs::write(path, snapshot.to_string()).unwrap();
    }

    /// Writes session `id` for the project dir into the account `dir`, with
    /// a change time `age` seconds ago.
    fn session(&self, dir: &Path, id: &str, age: u64) -> PathBuf {
        let path = dir.join(format!("projects/{}/{id}.jsonl", enc(&self.proj())));
        let text = format!(
            "{{\"type\":\"user\",\"sessionId\":\"{id}\",\"cwd\":\"{}\",\"message\":{{\"content\":\"hi\"}}}}\n",
            self.proj().display()
        );
        write(&path, &text);
        set_age(&path, age);
        path
    }

    fn org(&self, path: PathBuf, uuid: &str) {
        write(
            &path,
            &json!({"oauthAccount": {"organizationUuid": uuid}}).to_string(),
        );
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// The `projects/` name for an ASCII path (ADR 0006).
fn enc(path: &Path) -> String {
    path.to_str()
        .unwrap()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn set_age(path: &Path, secs: u64) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(secs))
        .unwrap();
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

/// The JSON line of the fake claude in the output of `on_tty`.
fn tty_json(output: &str) -> Value {
    let line = output
        .lines()
        .find_map(|l| l.find("{\"argv\"").map(|i| &l[i..]))
        .unwrap_or_else(|| panic!("no fake claude JSON: {output}"));
    serde_json::from_str(line).unwrap()
}

#[test]
fn resume_launches_the_owner_when_it_is_not_limited() {
    let fx = Fixture::new();
    // No ccx state names angel2, and main has more capacity.
    fx.session(&fx.angel2(), ID, 60);
    fx.usage("main", 0.0);
    fx.usage("angel2", 50.0);
    for args in [
        &["--resume", ID][..],
        &["-r", ID],
        &[&format!("--resume={ID}")],
    ] {
        let (json, out) = launched(&mut fx.ccx(args));
        assert_eq!(json["env"]["CCX_ACTIVE"], "angel2", "{args:?}");
        assert_eq!(json["argv"], json!(args), "{args:?}");
        assert_eq!(stderr(&out), "");
    }
    // ccx copied no files.
    assert!(!fx.main().join("projects").exists());
    // A launch of the owner is not a pick by usage.
    assert!(!fx.state().exists());
}

#[test]
fn continue_launches_the_account_with_the_newest_session_for_the_cwd() {
    let fx = Fixture::new();
    fx.session(&fx.main(), ID, 600);
    fx.session(&fx.angel2(), OTHER_ID, 60);
    // A newer session in a different directory does not count.
    write(
        &fx.main()
            .join(format!("projects/-elsewhere/{OTHER_ID}.jsonl")),
        "{}\n",
    );
    fx.usage("main", 0.0);
    fx.usage("angel2", 50.0);
    for flag in ["-c", "--continue"] {
        let (json, _) = launched(&mut fx.ccx(&[flag]));
        assert_eq!(json["env"]["CCX_ACTIVE"], "angel2");
        assert_eq!(json["argv"], json!([flag]));
    }

    // With no session for the cwd, `-c` uses automatic selection.
    let fx = Fixture::new();
    fx.usage("main", 50.0);
    fx.usage("angel2", 0.0);
    let (json, _) = launched(&mut fx.ccx(&["-c"]));
    assert_eq!(json["env"]["CCX_ACTIVE"], "angel2");
    assert_eq!(json["argv"], json!(["-c"]));
}

#[test]
fn a_limited_owner_hands_off_to_the_automatic_pick() {
    let fx = Fixture::new();
    let source = fx.session(&fx.main(), ID, 60);
    fx.usage("main", 92.0);
    fx.usage("angel2", 10.0);

    let (json, out) = launched(&mut fx.ccx(&["--model", "opus", "--resume", ID]));
    assert_eq!(json["env"]["CCX_ACTIVE"], "angel2");
    assert_eq!(json["argv"], json!(["--model", "opus", "--resume", ID]));
    // No banner in a pipe.
    assert_eq!(stderr(&out), "");
    let copy = fx.angel2().join(source.strip_prefix(fx.main()).unwrap());
    assert_eq!(fs::read(&copy).unwrap(), fs::read(&source).unwrap());
    let state: Value = serde_json::from_slice(&fs::read(fx.state()).unwrap()).unwrap();
    assert_eq!(state["last"], "angel2");

    // `-c` becomes `--resume <id>` in the target.
    let fx = Fixture::new();
    fx.session(&fx.main(), ID, 60);
    fx.usage("main", 92.0);
    fx.usage("angel2", 10.0);
    let (json, _) = launched(&mut fx.ccx(&["-c", "fix it"]));
    assert_eq!(json["env"]["CCX_ACTIVE"], "angel2");
    assert_eq!(json["argv"], json!(["--resume", ID, "fix it"]));
    assert!(fx.angel2().join("projects").exists());
}

#[test]
fn the_handoff_banner_names_owner_target_and_reason_on_a_tty() {
    let fx = Fixture::new();
    fx.session(&fx.main(), ID, 60);
    fx.usage("main", 92.0);
    fx.usage("angel2", 10.0);
    fx.org(fx.home.join(".claude.json"), "org-a");
    fx.org(fx.angel2().join(".claude.json"), "org-b");
    let output = on_tty(&fx.ccx(&[]), &["--resume", ID]);
    assert!(
        output.contains("ccx: handoff main → angel2 (main limited: 5h 92%)"),
        "{output}"
    );
    assert!(output.contains("different organizations"), "{output}");
    assert!(output.contains("ccx → angel2 · 5h 10%"), "{output}");
    assert_eq!(tty_json(&output)["env"]["CCX_ACTIVE"], "angel2");
}

#[test]
fn a_different_copy_in_the_target_gives_a_fork() {
    let fx = Fixture::new();
    let source = fx.session(&fx.main(), ID, 60);
    let theirs = format!("{{\"sessionId\":\"{ID}\",\"type\":\"user\"}}\n");
    let copy = fx.angel2().join(source.strip_prefix(fx.main()).unwrap());
    write(&copy, &theirs);
    set_age(&copy, 600);
    fx.usage("main", 92.0);
    fx.usage("angel2", 10.0);

    let output = on_tty(&fx.ccx(&[]), &["--resume", ID]);
    let json = tty_json(&output);
    assert_eq!(json["env"]["CCX_ACTIVE"], "angel2");
    let new = json["argv"][1].as_str().unwrap().to_string();
    assert_eq!(json["argv"], json!(["--resume", new]));
    assert_ne!(new, ID);
    assert!(
        output.contains(&format!(
            "angel2 had a different copy, new session id {new}"
        )),
        "{output}"
    );
    // The copy in the target stays the same. The fork has the new id.
    assert_eq!(fs::read_to_string(&copy).unwrap(), theirs);
    let fork = copy.with_file_name(format!("{new}.jsonl"));
    assert!(fs::read_to_string(fork).unwrap().contains(&new));
}

#[test]
fn all_limited_launches_the_owner_with_a_warning() {
    let fx = Fixture::new();
    fx.session(&fx.main(), ID, 60);
    fx.usage("main", 92.0);
    fx.usage("angel2", 95.0);
    let (json, out) = launched(&mut fx.ccx(&["--resume", ID]));
    assert_eq!(json["env"]["CCX_ACTIVE"], "main");
    assert_eq!(json["argv"], json!(["--resume", ID]));
    assert_eq!(stderr(&out), "");
    assert!(!fx.angel2().join("projects").exists());

    let output = on_tty(&fx.ccx(&[]), &["--resume", ID]);
    assert!(
        output.contains("ccx: warning: all accounts limited; the session stays in main"),
        "{output}"
    );
    assert_eq!(tty_json(&output)["env"]["CCX_ACTIVE"], "main");
}

#[test]
fn a_bare_resume_uses_automatic_selection_and_no_handoff() {
    let fx = Fixture::new();
    fx.session(&fx.main(), ID, 60);
    fx.usage("main", 92.0);
    fx.usage("angel2", 10.0);
    for args in [&["--resume"][..], &["--resume", "search words"]] {
        let (json, _) = launched(&mut fx.ccx(args));
        assert_eq!(json["env"]["CCX_ACTIVE"], "angel2", "{args:?}");
        assert_eq!(json["argv"], json!(args));
    }
    assert!(!fx.angel2().join("projects").exists());
    let state: Value = serde_json::from_slice(&fs::read(fx.state()).unwrap()).unwrap();
    assert_eq!(state["last"], "angel2");
}

#[test]
fn a_named_account_never_hands_off() {
    let fx = Fixture::new();
    fx.session(&fx.main(), ID, 60);
    fx.usage("main", 99.0);
    fx.usage("angel2", 0.0);
    for args in [&["@main", "-c"][..], &["@main", "--resume", ID]] {
        let (json, _) = launched(&mut fx.ccx(args));
        assert_eq!(json["env"]["CCX_ACTIVE"], "main", "{args:?}");
        assert_eq!(json["argv"], json!(args[1..]));
    }
    let (json, _) = launched(fx.ccx(&["-c"]).env("CCX_ACCOUNT", "main"));
    assert_eq!(json["env"]["CCX_ACTIVE"], "main");
    assert!(!fx.angel2().join("projects").exists());
}

#[test]
fn a_refused_handoff_keeps_the_session_in_the_owner() {
    let fx = Fixture::new();
    // A change less than 5 seconds ago: the session is probably open.
    fx.session(&fx.main(), ID, 0);
    fx.usage("main", 92.0);
    fx.usage("angel2", 10.0);
    let output = on_tty(&fx.ccx(&[]), &["--resume", ID]);
    assert!(
        output.contains("ccx: warning: cannot hand off to angel2: ")
            && output.contains("the session stays in main"),
        "{output}"
    );
    assert_eq!(tty_json(&output)["env"]["CCX_ACTIVE"], "main");
    assert!(!fx.angel2().join("projects").exists());
}

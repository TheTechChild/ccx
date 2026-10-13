//! End-to-end tests for `ccx handoff`. Each test uses a temp HOME with two
//! account directories and `testdata/fakebin/claude`, which prints its
//! argv, cwd and env as JSON.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, SystemTime};

use serde_json::Value;

const CCX: &str = env!("CARGO_BIN_EXE_ccx");

const CONFIG: &str = r#"
[accounts.main]
dir = "~/.claude"

[accounts.angel2]
dir = "~/.claude-angel2"
"#;

const ID: &str = "11111111-1111-4111-8111-111111111111";
const OTHER_ID: &str = "22222222-2222-4222-8222-222222222222";
const ELSEWHERE_ID: &str = "33333333-3333-4333-8333-333333333333";

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
        fs::create_dir_all(home.join("work/other")).unwrap();
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

    /// Writes a full session `id` for `cwd` into the account `dir`, with a
    /// change time one minute ago.
    fn session(&self, dir: &Path, id: &str, cwd: &Path, prompt: &str) -> PathBuf {
        let enc = enc(cwd);
        let transcript = dir.join(format!("projects/{enc}/{id}.jsonl"));
        let lines = [
            format!(
                r#"{{"type":"user","sessionId":"{id}","cwd":"{}","slug":"staged-beacon","message":{{"role":"user","content":"{prompt}"}}}}"#,
                cwd.display()
            ),
            format!(r#"{{"type":"assistant","sessionId":"{id}","slug":"staged-beacon"}}"#),
        ];
        write(&transcript, &(lines.join("\n") + "\n"));
        set_old(&transcript);
        transcript
    }

    /// Adds the other files of session `id` to the account `dir`.
    fn sidecars(&self, dir: &Path, id: &str) {
        let other = enc(&self.home.join("work/other"));
        let enc = enc(&self.proj());
        write(
            &dir.join(format!("projects/{enc}/{id}/subagents/agent-1.jsonl")),
            &format!("{{\"sessionId\":\"{id}\",\"agentId\":\"1\"}}\n"),
        );
        write(
            &dir.join(format!("projects/{enc}/{id}/subagents/agent-1.meta.json")),
            "{}",
        );
        write(
            &dir.join(format!("projects/{other}/{id}/tool-results/r.txt")),
            "result",
        );
        write(
            &dir.join(format!("projects/{enc}/memory/MEMORY.md")),
            "memory",
        );
        write(
            &dir.join(format!("file-history/{id}/0123456789abcdef@v1")),
            "v1",
        );
        fs::create_dir_all(dir.join(format!("session-env/{id}"))).unwrap();
        write(&dir.join("plans/staged-beacon.md"), "the plan");
        write(&dir.join("plans/unrelated.md"), "other plan");
        write(&dir.join(format!("todos/{id}-agent-{id}.json")), "[]");
        write(&dir.join(format!("jobs/{}/x", &id[..8])), "job");
    }
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

fn set_old(path: &Path) {
    let time = SystemTime::now() - Duration::from_secs(60);
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(time)
        .unwrap();
}

/// All files under `dir`, relative, with their contents.
fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap().display().to_string();
            if path.is_dir() {
                out.insert(format!("{rel}/"), Vec::new());
                walk(root, &path, out);
            } else {
                out.insert(rel, fs::read(&path).unwrap_or_default());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn run(cmd: &mut Command) -> Output {
    cmd.output().expect("run ccx")
}

fn ok(cmd: &mut Command) -> Output {
    let out = run(cmd);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    out
}

fn ps_start(pid: u32) -> String {
    let out = Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .env("TZ", "UTC")
        .env("LC_ALL", "C")
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
fn list_shows_the_threads_for_the_cwd_from_all_accounts() {
    let fx = Fixture::new();
    fx.session(&fx.main(), ID, &fx.proj(), "fix the bug");
    fx.session(&fx.angel2(), OTHER_ID, &fx.proj(), "write the docs");
    let other = fx.home.join("work/other");
    fx.session(&fx.main(), ELSEWHERE_ID, &other, "other dir");

    let out = stdout(&ok(&mut fx.ccx(&["handoff"])));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    assert!(out.contains(ID) && out.contains("fix the bug"), "{out}");
    assert!(
        out.contains(OTHER_ID) && out.contains("write the docs"),
        "{out}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with(ID) && l.contains(" main "))
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with(OTHER_ID) && l.contains(" angel2 "))
    );
    assert!(!out.contains(ELSEWHERE_ID));

    let out = stdout(&ok(&mut fx.ccx(&["handoff", "--all"])));
    assert_eq!(out.lines().count(), 3, "{out}");
    assert!(
        out.contains(ELSEWHERE_ID) && out.contains("~/work/other"),
        "{out}"
    );
}

#[test]
fn handoff_copies_the_file_set_and_resumes_in_the_target() {
    let fx = Fixture::new();
    fx.session(&fx.main(), ID, &fx.proj(), "fix the bug");
    fx.sidecars(&fx.main(), ID);
    let before = tree(&fx.main());

    let out = ok(&mut fx.ccx(&["handoff", ID, "@angel2"]));
    let json: Value = serde_json::from_slice(&out.stdout).expect("fake claude JSON");
    assert_eq!(json["argv"], serde_json::json!(["--resume", ID]));
    assert_eq!(
        json["env"]["CLAUDE_CONFIG_DIR"],
        fx.angel2().to_str().unwrap()
    );
    assert_eq!(json["env"]["CCX_ACTIVE"], "angel2");
    assert_eq!(json["cwd"], fx.proj().to_str().unwrap());
    assert!(stderr(&out).contains(&format!("copied session {ID} from main to angel2")));

    let other = enc(&fx.home.join("work/other"));
    let enc = enc(&fx.proj());
    let have: Vec<String> = tree(&fx.angel2()).into_keys().collect();
    let want = [
        "file-history/".to_string(),
        format!("file-history/{ID}/"),
        format!("file-history/{ID}/0123456789abcdef@v1"),
        "plans/".into(),
        "plans/staged-beacon.md".into(),
        "projects/".into(),
        format!("projects/{enc}/"),
        format!("projects/{enc}/{ID}/"),
        format!("projects/{enc}/{ID}/subagents/"),
        format!("projects/{enc}/{ID}/subagents/agent-1.jsonl"),
        format!("projects/{enc}/{ID}/subagents/agent-1.meta.json"),
        format!("projects/{enc}/{ID}.jsonl"),
        format!("projects/{other}/"),
        format!("projects/{other}/{ID}/"),
        format!("projects/{other}/{ID}/tool-results/"),
        format!("projects/{other}/{ID}/tool-results/r.txt"),
        "session-env/".into(),
        format!("session-env/{ID}/"),
    ];
    let mut want: Vec<String> = want.into_iter().collect();
    want.sort();
    assert_eq!(have, want);
    let copied = tree(&fx.angel2());
    for (path, bytes) in &copied {
        assert_eq!(Some(bytes), before.get(path), "{path}");
    }
    assert_eq!(tree(&fx.main()), before, "the source changed");
}

#[test]
fn a_prefix_copy_is_overwritten() {
    let fx = Fixture::new();
    let source = fx.session(&fx.main(), ID, &fx.proj(), "fix the bug");
    let text = fs::read_to_string(&source).unwrap();
    let first = text.lines().next().unwrap().to_string() + "\n";
    let target = fx.angel2().join(source.strip_prefix(fx.main()).unwrap());
    write(&target, &first);

    let out = ok(&mut fx.ccx(&["handoff", ID, "@angel2", "--no-resume"]));
    assert_eq!(stdout(&out).trim(), ID);
    assert_eq!(fs::read_to_string(&target).unwrap(), text);
}

#[test]
fn a_different_copy_gets_a_fork_with_a_new_id() {
    let fx = Fixture::new();
    let source = fx.session(&fx.main(), ID, &fx.proj(), "fix the bug");
    fx.sidecars(&fx.main(), ID);
    let target = fx.angel2().join(source.strip_prefix(fx.main()).unwrap());
    let theirs =
        format!("{{\"sessionId\":\"{ID}\",\"type\":\"user\",\"slug\":\"staged-beacon\"}}\n");
    write(&target, &theirs);
    write(&fx.angel2().join("plans/staged-beacon.md"), "their plan");
    let before = tree(&fx.main());

    let out = ok(&mut fx.ccx(&["handoff", ID, "@angel2", "--no-resume"]));
    let new = stdout(&out).trim().to_string();
    assert_eq!(new.len(), 36);
    assert_ne!(new, ID);
    let err = stderr(&out);
    assert!(
        err.contains(&format!("wrote a fork with the new session id {new}")),
        "{err}"
    );
    assert!(err.contains("plans/staged-beacon.md"), "{err}");

    // The target copy and its plan stay the same.
    assert_eq!(fs::read_to_string(&target).unwrap(), theirs);
    assert_eq!(
        fs::read_to_string(fx.angel2().join("plans/staged-beacon.md")).unwrap(),
        "their plan"
    );
    // The fork has the new id in each file name and in each line.
    let enc = enc(&fx.proj());
    let fork = fs::read_to_string(fx.angel2().join(format!("projects/{enc}/{new}.jsonl"))).unwrap();
    let want = fs::read_to_string(&source).unwrap().replace(ID, &new);
    assert_eq!(fork, want);
    assert!(!fork.contains(ID));
    let agent = fs::read_to_string(
        fx.angel2()
            .join(format!("projects/{enc}/{new}/subagents/agent-1.jsonl")),
    )
    .unwrap();
    assert!(agent.contains(&format!("\"sessionId\":\"{new}\"")));
    assert!(fx.angel2().join(format!("file-history/{new}")).is_dir());
    assert!(fx.angel2().join(format!("session-env/{new}")).is_dir());
    assert_eq!(tree(&fx.main()), before, "the source changed");

    // The resume uses the fork id.
    let out = ok(&mut fx.ccx(&["handoff", ID, "@angel2"]));
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["argv"][0], "--resume");
    assert_ne!(json["argv"][1], ID);
}

#[test]
fn an_open_session_is_refused() {
    let fx = Fixture::new();
    fx.session(&fx.main(), ID, &fx.proj(), "fix the bug");
    let me = std::process::id();
    let file = fx.main().join(format!("sessions/{me}.json"));
    let records = [
        format!(
            r#"{{"pid":{me},"sessionId":"{ID}","procStart":"{}"}}"#,
            ps_start(me)
        ),
        format!(r#"{{"pid":{me},"sessionId":"{ID}"}}"#),
    ];
    for record in records {
        write(&file, &record);
        let out = run(&mut fx.ccx(&["handoff", ID, "@angel2"]));
        assert!(!out.status.success());
        assert!(
            stderr(&out).contains(&format!("session {ID} is open in account main (pid {me})")),
            "{}",
            stderr(&out)
        );
        assert!(out.stdout.is_empty());
        assert!(!fx.angel2().exists());
    }
}

#[test]
fn stale_session_files_and_key_files_do_not_refuse() {
    use std::os::unix::fs::PermissionsExt;
    let fx = Fixture::new();
    fx.session(&fx.main(), ID, &fx.proj(), "fix the bug");
    let mut child = Command::new("true").spawn().unwrap();
    let dead = child.id();
    child.wait().unwrap();
    let me = std::process::id();
    let sessions = fx.main().join("sessions");
    write(
        &sessions.join(format!("{dead}.json")),
        &format!(r#"{{"pid":{dead},"sessionId":"{ID}"}}"#),
    );
    write(
        &sessions.join("1.json"),
        &format!(r#"{{"pid":{me},"sessionId":"{ID}","procStart":"Thu Jan  1 00:00:00 1970"}}"#),
    );
    // A `.key` file that would refuse the handoff if ccx read it. Its mode
    // is 0000, so a read would also fail.
    let key = sessions.join(format!("{me}.{}.key", "ab".repeat(32)));
    write(&key, &format!(r#"{{"pid":{me},"sessionId":"{ID}"}}"#));
    fs::set_permissions(&key, fs::Permissions::from_mode(0o000)).unwrap();

    let out = ok(&mut fx.ccx(&["handoff", ID, "@angel2", "--no-resume"]));
    assert_eq!(stdout(&out).trim(), ID);
    fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn a_recent_change_is_refused() {
    let fx = Fixture::new();
    let source = fx.session(&fx.main(), ID, &fx.proj(), "fix the bug");
    fs::File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_modified(SystemTime::now())
        .unwrap();
    let out = run(&mut fx.ccx(&["handoff", ID, "@angel2"]));
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("changed in the last 5 seconds in account main"),
        "{}",
        stderr(&out)
    );
    assert!(!fx.angel2().exists());
}

#[test]
fn different_organizations_show_a_warning() {
    let fx = Fixture::new();
    fx.session(&fx.main(), ID, &fx.proj(), "fix the bug");
    let org = |uuid: &str| format!(r#"{{"oauthAccount":{{"organizationUuid":"{uuid}"}}}}"#);
    write(&fx.home.join(".claude.json"), &org("org-a"));
    write(&fx.angel2().join(".claude.json"), &org("org-b"));
    let out = ok(&mut fx.ccx(&["handoff", ID, "@angel2", "--no-resume"]));
    assert!(
        stderr(&out).contains("different organizations"),
        "{}",
        stderr(&out)
    );

    write(&fx.angel2().join(".claude.json"), &org("org-a"));
    let out = ok(&mut fx.ccx(&["handoff", ID, "@angel2", "--no-resume"]));
    assert!(!stderr(&out).contains("different organizations"));

    write(&fx.angel2().join(".claude.json"), "{}");
    let out = ok(&mut fx.ccx(&["handoff", ID, "@angel2", "--no-resume"]));
    assert!(!stderr(&out).contains("different organizations"));
}

#[test]
fn errors_for_a_bad_id_or_target() {
    let fx = Fixture::new();
    fx.session(&fx.main(), ID, &fx.proj(), "fix the bug");
    let fail = |args: &[&str]| {
        let out = run(&mut fx.ccx(args));
        assert!(!out.status.success(), "{args:?}");
        stderr(&out)
    };
    assert!(fail(&["handoff", ID]).contains("name the target account"));
    assert!(fail(&["handoff", ID, "@nope"]).contains("unknown account `nope`"));
    assert!(fail(&["handoff", OTHER_ID, "@angel2"]).contains("no session"));
    assert!(fail(&["handoff", "../x", "@angel2"]).contains("is not a session id"));
    assert!(fail(&["handoff", ID, "@main"]).contains("only in account main"));

    // A unique start of the id is enough.
    let out = ok(&mut fx.ccx(&["handoff", &ID[..8], "@angel2", "--no-resume"]));
    assert_eq!(stdout(&out).trim(), ID);
}

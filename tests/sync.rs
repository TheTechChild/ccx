//! End-to-end tests for `ccx manage sync` and the health check of
//! `ccx @name`. Each test uses a temp HOME with a synthetic copy of the
//! account layout and the fake claude in `testdata/fakebin`.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CCX: &str = env!("CARGO_BIN_EXE_ccx");

const CONFIG: &str = "\
[accounts.main]
dir = \"~/.claude\"

[accounts.angel2]
dir = \"~/.claude-angel2\"
";

struct Fixture {
    home: tempfile::TempDir,
}

impl Fixture {
    /// HOME with the config, an empty base dir and an empty angel2 dir.
    fn new(config: &str) -> Fixture {
        let home = tempfile::tempdir().expect("temp HOME");
        let fx = Fixture { home };
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
        let fakebin = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fakebin");
        let mut cmd = Command::new(CCX);
        cmd.args(args)
            .env_clear()
            .env("HOME", self.home())
            .env("PATH", format!("{}:/usr/bin:/bin", fakebin.display()));
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.ccx(args).output().expect("run ccx")
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(out.status.success(), "{}", text(&out));
        String::from_utf8(out.stdout).unwrap()
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn text(out: &Output) -> String {
    format!(
        "status: {}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Each path under `root` (never through a symlink) with its kind and
/// content: the bytes of a file, the target of a link, or "dir".
fn snapshot(root: &Path) -> BTreeMap<PathBuf, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path.strip_prefix(root).unwrap().to_path_buf();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.file_type().is_symlink() {
                let target = fs::read_link(&path).unwrap();
                out.insert(rel, format!("link {}", target.display()));
            } else if meta.is_dir() {
                out.insert(rel, "dir".into());
                walk(root, &path, out);
            } else {
                let bytes = fs::read(&path).unwrap();
                out.insert(rel, format!("file {bytes:?}"));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn link_target(path: &Path) -> PathBuf {
    fs::read_link(path).unwrap_or_else(|e| panic!("{} is not a link: {e}", path.display()))
}

/// A synthetic copy of the real layout: the base entries, the skills (real
/// dirs, links to other repos, `synced` and a dotfile) and an angel2 dir
/// that already links `settings.json`, `CLAUDE.md` and `plugins`.
fn real_layout(fx: &Fixture) {
    let base = fx.base();
    for name in [
        ".claude.json",
        ".DS_Store",
        ".gitignore",
        ".last-cleanup",
        "CLAUDE.md",
        "README.md",
        "daemon.log",
        "gh-pr-status-cache.json",
        "history.jsonl",
        "mcp-needs-auth-cache.json",
        "settings.json",
        "settings.json.bak-ultracode",
        "settings.local.json",
        "statusline.py",
    ] {
        write(&base.join(name), name);
    }
    for name in [
        ".claude.json.lock",
        ".git",
        "backups",
        "cache",
        "chrome",
        "daemon",
        "downloads",
        "feedback",
        "file-history",
        "ide",
        "jobs",
        "paste-cache",
        "plans",
        "projects",
        "session-env",
        "sessions",
        "shell-snapshots",
        "state",
        "telemetry",
    ] {
        fs::create_dir(base.join(name)).unwrap();
    }
    write(&base.join("plugins/installed.json"), "{}");
    write(&base.join("plugins/synced/org.json"), "per org");
    write(&base.join("skills/.DS_Store"), "x");
    write(&base.join("skills/debug-sigil-character/SKILL.md"), "real");
    write(&base.join("skills/synced/x/SKILL.md"), "base synced");
    let repo = fx.home().join("projects/skills");
    for name in ["tdd", "to-issues", "deploy"] {
        write(&repo.join(name).join("SKILL.md"), name);
        symlink(repo.join(name), base.join("skills").join(name)).unwrap();
    }

    let acct = fx.angel2();
    for name in ["settings.json", "CLAUDE.md", "plugins"] {
        symlink(base.join(name), acct.join(name)).unwrap();
    }
    for name in [".claude.json", ".last-cleanup", "history.jsonl"] {
        write(&acct.join(name), "angel2");
    }
    for name in ["backups", "cache", "projects", "sessions"] {
        fs::create_dir(acct.join(name)).unwrap();
    }
    write(&acct.join("skills/synced/y/SKILL.md"), "angel2 synced");
}

#[test]
fn sync_on_the_real_layout_adds_skill_links_and_keeps_correct_links() {
    let fx = Fixture::new(CONFIG);
    real_layout(&fx);
    let base = fx.base();
    let acct = fx.angel2();
    let synced_before = snapshot(&acct.join("skills/synced"));

    let out = fx.ok(&["manage", "sync", "angel2"]);

    for name in ["settings.json", "CLAUDE.md", "plugins"] {
        assert!(out.contains(&format!("  OK        {name}\n")), "{out}");
        assert_eq!(link_target(&acct.join(name)), base.join(name));
    }
    assert!(out.contains("  LINK      statusline.py"), "{out}");
    assert!(out.contains("  OK        skills\n"), "{out}");
    for name in ["debug-sigil-character", "deploy", "tdd", "to-issues"] {
        let path = format!("skills/{name}");
        assert!(out.contains(&format!("  LINK      {path} -> ")), "{out}");
        assert_eq!(link_target(&acct.join(&path)), base.join(&path));
    }
    assert!(!out.contains("skills/synced"), "{out}");
    assert!(!out.contains(".DS_Store"), "{out}");
    assert_eq!(snapshot(&acct.join("skills/synced")), synced_before);
    // Each unknown base entry is in the plan, and only those.
    let unknown: Vec<&str> = out
        .lines()
        .filter_map(|l| l.strip_prefix("  UNKNOWN   "))
        .collect();
    assert_eq!(
        unknown,
        [
            "README.md (not shared)",
            "gh-pr-status-cache.json (not shared)",
            "settings.json.bak-ultracode (not shared)",
        ]
    );
    // Never-share entries are not linked.
    for name in [
        "history.jsonl",
        "projects",
        "settings.local.json",
        "chrome",
        "daemon.log",
        "downloads",
        "feedback",
    ] {
        let meta = fs::symlink_metadata(acct.join(name));
        assert!(
            meta.is_err() || !meta.unwrap().file_type().is_symlink(),
            "{name}"
        );
    }
    assert!(out.contains("Sync made 5 changes."), "{out}");

    // A second run changes nothing.
    let before = snapshot(fx.home());
    let again = fx.ok(&["manage", "sync", "angel2"]);
    assert!(again.contains("Sync made 0 changes."), "{again}");
    assert_eq!(snapshot(fx.home()), before);
}

#[test]
fn dry_run_changes_no_file() {
    let fx = Fixture::new(CONFIG);
    real_layout(&fx);
    // Add one item of each kind of change.
    write(&fx.angel2().join("statusline.py"), "conflict");
    fs::remove_file(fx.angel2().join("plugins")).unwrap();
    symlink("/elsewhere/plugins", fx.angel2().join("plugins")).unwrap();
    let before = snapshot(fx.home());

    let out = fx.ok(&["manage", "sync", "angel2", "--dry-run", "--force"]);
    assert!(out.contains("  CONFLICT  statusline.py"), "{out}");
    assert!(out.contains("  RELINK    plugins"), "{out}");
    assert!(out.contains("  LINK      skills/tdd"), "{out}");
    assert!(out.ends_with("Dry run. Nothing changed.\n"), "{out}");
    assert_eq!(snapshot(fx.home()), before);
}

#[test]
fn force_with_a_linked_plugins_dir_keeps_base_plugins_byte_for_byte() {
    let fx = Fixture::new(CONFIG);
    real_layout(&fx);
    // Conflicts next to the plugins link: a real file and a real dir.
    write(&fx.angel2().join("statusline.py"), "angel2 copy");
    write(&fx.angel2().join("skills/tdd/SKILL.md"), "angel2 tdd");
    let base_before = snapshot(&fx.base());

    let out = fx.ok(&["manage", "sync", "angel2", "--force"]);
    assert!(out.contains("  OK        plugins\n"), "{out}");
    assert!(out.contains("  CONFLICT  statusline.py"), "{out}");
    assert!(out.contains("  CONFLICT  skills/tdd"), "{out}");
    assert_eq!(snapshot(&fx.base()), base_before);
    assert_eq!(snapshot(&fx.base().join("plugins")), {
        let mut want = BTreeMap::new();
        want.insert(PathBuf::from("installed.json"), format!("file {:?}", b"{}"));
        want.insert(PathBuf::from("synced"), "dir".into());
        want.insert(
            PathBuf::from("synced/org.json"),
            format!("file {:?}", b"per org"),
        );
        want
    });
}

#[test]
fn conflict_moves_to_a_backup_with_its_content() {
    let fx = Fixture::new(CONFIG);
    let base = fx.base();
    let acct = fx.angel2();
    write(&base.join("settings.json"), "base");
    write(&base.join("agents/a.md"), "base agent");
    // settings.json became a real file, for example after a `/config` write.
    write(&acct.join("settings.json"), "angel2 settings");
    write(&acct.join("agents/mine.md"), "angel2 agent");

    // No terminal and no --force: CONFLICT items stay, and the exit is 1.
    let out = fx.run(&["manage", "sync", "angel2"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("  CONFLICT  settings.json (real file"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  CONFLICT  agents (real directory"),
        "{stdout}"
    );
    assert!(
        stdout.contains("2 CONFLICT items did not change"),
        "{stdout}"
    );
    assert!(stdout.contains("--force"), "{stdout}");
    assert_eq!(
        fs::read_to_string(acct.join("settings.json")).unwrap(),
        "angel2 settings"
    );
    assert!(!acct.join(".ccx-backup").exists());

    // With --force, the items move to .ccx-backup/<timestamp>/.
    let out = fx.ok(&["manage", "sync", "angel2", "--force"]);
    assert!(out.contains("moved the CONFLICT items to"), "{out}");
    let backups: Vec<_> = fs::read_dir(acct.join(".ccx-backup"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(backups.len(), 1);
    let stamp = backups[0]
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(stamp.len(), 16, "{stamp}");
    assert!(
        stamp.ends_with('Z') && stamp.as_bytes()[8] == b'T',
        "{stamp}"
    );
    assert_eq!(
        fs::read_to_string(backups[0].join("settings.json")).unwrap(),
        "angel2 settings"
    );
    assert_eq!(
        fs::read_to_string(backups[0].join("agents/mine.md")).unwrap(),
        "angel2 agent"
    );
    assert_eq!(
        link_target(&acct.join("settings.json")),
        base.join("settings.json")
    );
    assert_eq!(link_target(&acct.join("agents")), base.join("agents"));
    assert_eq!(
        fs::read_to_string(base.join("settings.json")).unwrap(),
        "base"
    );
}

#[test]
fn own_items_are_skipped() {
    let fx = Fixture::new(&format!(
        "{CONFIG}own = [\"settings.json\", \"skills/tdd\"]\n"
    ));
    write(&fx.base().join("settings.json"), "base");
    fs::create_dir_all(fx.base().join("skills/tdd")).unwrap();
    write(&fx.angel2().join("settings.json"), "mine");
    let out = fx.ok(&["manage", "sync", "angel2"]);
    assert!(out.contains("  SKIP      settings.json (own)"), "{out}");
    assert!(out.contains("  SKIP      skills/tdd (own)"), "{out}");
    assert_eq!(
        fs::read_to_string(fx.angel2().join("settings.json")).unwrap(),
        "mine"
    );
    assert!(!fx.angel2().join("skills/tdd").exists());
}

#[test]
fn sync_refuses_the_base_account_and_unknown_names() {
    let fx = Fixture::new(CONFIG);
    let out = fx.run(&["manage", "sync", "main"]);
    assert!(!out.status.success());
    assert!(
        text(&out).contains("sync never changes the base account"),
        "{}",
        text(&out)
    );
    let out = fx.run(&["manage", "sync", "nobody"]);
    assert!(!out.status.success());
    assert!(
        text(&out).contains("known accounts: angel2, main"),
        "{}",
        text(&out)
    );
    // A name or --all is necessary.
    assert!(!fx.run(&["manage", "sync"]).status.success());
}

#[test]
fn sync_refuses_paths_in_the_base_dir() {
    // The account dir is a link to the base dir: each path resolves into
    // the base dir.
    let fx = Fixture::new(&format!(
        "{CONFIG}\n[accounts.evil]\ndir = \"~/.claude-evil\"\n"
    ));
    write(&fx.base().join("settings.json"), "base");
    write(&fx.base().join("skills/tdd/SKILL.md"), "tdd");
    symlink(fx.base(), fx.home().join(".claude-evil")).unwrap();
    let before = snapshot(&fx.base());
    for args in [
        &["manage", "sync", "evil", "--force"][..],
        &["manage", "sync", "evil", "--dry-run"][..],
    ] {
        let out = fx.run(args);
        assert!(!out.status.success(), "{}", text(&out));
        assert!(
            text(&out).contains("inside the base directory"),
            "{}",
            text(&out)
        );
        assert_eq!(snapshot(&fx.base()), before);
    }
}

#[test]
fn sync_refuses_paths_outside_the_account_dir() {
    let fx = Fixture::new(CONFIG);
    write(&fx.base().join("settings.json"), "base");
    write(&fx.angel2().join("settings.json"), "angel2");
    // The backup dir is a link to a dir outside the account.
    let outside = fx.home().join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, fx.angel2().join(".ccx-backup")).unwrap();
    let before = snapshot(fx.home());

    let out = fx.run(&["manage", "sync", "angel2", "--force"]);
    assert!(!out.status.success(), "{}", text(&out));
    assert!(
        text(&out).contains("outside the account directory"),
        "{}",
        text(&out)
    );
    assert_eq!(snapshot(fx.home()), before);
}

#[test]
fn all_syncs_each_account_but_the_base() {
    let fx = Fixture::new(&format!(
        "{CONFIG}\n[accounts.work]\ndir = \"~/.claude-work\"\n"
    ));
    fs::create_dir(fx.home().join(".claude-work")).unwrap();
    write(&fx.base().join("CLAUDE.md"), "base");
    let out = fx.ok(&["manage", "sync", "--all"]);
    assert!(out.contains("angel2 ("), "{out}");
    assert!(out.contains("work ("), "{out}");
    assert!(!out.contains("main ("), "{out}");
    for dir in [fx.angel2(), fx.home().join(".claude-work")] {
        assert_eq!(
            link_target(&dir.join("CLAUDE.md")),
            fx.base().join("CLAUDE.md")
        );
    }
    assert!(
        fs::symlink_metadata(fx.base().join("CLAUDE.md"))
            .unwrap()
            .is_file()
    );
}

#[test]
fn launch_health_check_adds_a_missing_skill_and_leaves_a_conflict() {
    let fx = Fixture::new(CONFIG);
    real_layout(&fx);
    let acct = fx.angel2();
    write(&acct.join("statusline.py"), "angel2 copy");
    write(&fx.base().join("skills/new-skill/SKILL.md"), "new");

    // A dry run starts nothing and changes nothing.
    let before = snapshot(fx.home());
    let out = fx
        .ccx(&["@angel2"])
        .env("CCX_DRY_RUN", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(snapshot(fx.home()), before);

    let out = fx.run(&["@angel2", "--version"]);
    assert!(out.status.success(), "{}", text(&out));
    let launched: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(launched["argv"], serde_json::json!(["--version"]));
    assert_eq!(
        link_target(&acct.join("skills/new-skill")),
        fx.base().join("skills/new-skill")
    );
    assert_eq!(
        link_target(&acct.join("skills/tdd")),
        fx.base().join("skills/tdd")
    );
    assert_eq!(
        fs::read_to_string(acct.join("statusline.py")).unwrap(),
        "angel2 copy"
    );
    assert!(!acct.join(".ccx-backup").exists());
}

#[test]
fn launch_of_the_base_account_changes_nothing() {
    let fx = Fixture::new(CONFIG);
    real_layout(&fx);
    let before = snapshot(fx.home());
    let out = fx.run(&["@main", "--version"]);
    assert!(out.status.success(), "{}", text(&out));
    // The launch records only its time in the ccx state (#22).
    let mut after = snapshot(fx.home());
    after.retain(|path, _| !path.starts_with(".local"));
    assert_eq!(after, before);
    assert!(fx.home().join(".local/state/ccx/launches/main").is_file());
}

#[test]
fn without_a_terminal_or_with_yes_sync_does_not_ask_and_shows_unknown_items() {
    let fx = Fixture::new(CONFIG);
    write(&fx.base().join("CLAUDE.md"), "base");
    write(&fx.base().join("notes/a.md"), "x");
    let config = fs::read_to_string(fx.home().join(".config/ccx/config.toml")).unwrap();
    for args in [
        &["manage", "sync", "--all"][..],
        &["manage", "sync", "--all", "-y"],
        &["manage", "sync", "angel2", "--yes", "--dry-run"],
    ] {
        let out = fx.ok(args);
        assert!(out.contains("UNKNOWN   notes (not shared)"), "{out}");
    }
    assert_eq!(
        fs::read_to_string(fx.home().join(".config/ccx/config.toml")).unwrap(),
        config
    );
    assert!(fs::symlink_metadata(fx.angel2().join("notes")).is_err());
}

#[test]
fn share_items_link_ignore_items_hide_and_launch_adds_a_missing_share_link() {
    let fx = Fixture::new(&format!(
        "{CONFIG}\n[sync]\nshare = [\"notes\", \"not-there\"]\nignore = [\"cache.json\"]\n"
    ));
    write(&fx.base().join("notes/a.md"), "x");
    write(&fx.base().join("cache.json"), "{}");
    let out = fx.ok(&["manage", "sync", "angel2"]);
    assert!(out.contains("LINK      notes"), "{out}");
    assert!(!out.contains("UNKNOWN"), "{out}");
    assert!(!out.contains("not-there"), "{out}");
    assert_eq!(
        link_target(&fx.angel2().join("notes")),
        fx.base().join("notes")
    );
    assert!(fs::symlink_metadata(fx.angel2().join("cache.json")).is_err());

    fs::remove_file(fx.angel2().join("notes")).unwrap();
    let out = fx.run(&["@angel2", "--version"]);
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        link_target(&fx.angel2().join("notes")),
        fx.base().join("notes")
    );
}

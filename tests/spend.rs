//! End-to-end tests for `ccx manage spend` (#24). Each test uses a temp
//! HOME with the synthetic transcripts in `testdata/spend` and `TZ=UTC`.
//! The PATH holds only trap commands: each one writes a marker file when
//! it runs. No test touches a real account, a real keychain or a real key.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const CCX: &str = env!("CARGO_BIN_EXE_ccx");

/// Commands that the spend report must never run: the secret stores,
/// claude and network tools.
const TRAPS: [&str; 6] = ["security", "secret-tool", "claude", "curl", "wget", "nc"];

const CONFIG: &str = r#"[accounts.main]
dir = "~/.claude"

[accounts.paid]
dir = "~/.claude-paid"
kind = "api_key"
key_store = "keyring"

[accounts.shell]
dir = "~/.claude-shell"
kind = "api_key"
key_command = "touch \"$HOME/trap-key_command\""

[pricing.claude-sonnet-4-5]
input = 2
output = 10
cache_write = 2.5
cache_read = 0.2
"#;

struct Fixture {
    home: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Fixture {
        let home = tempfile::tempdir().expect("temp HOME");
        let fx = Fixture { home };
        let data = testdata("spend");
        copy_dir(&data.join("main"), &fx.home().join(".claude"));
        copy_dir(&data.join("paid"), &fx.home().join(".claude-paid"));
        fs::create_dir_all(fx.home().join(".claude-shell")).unwrap();
        fs::create_dir_all(fx.home().join(".config/ccx")).unwrap();
        fs::write(fx.home().join(".config/ccx/config.toml"), CONFIG).unwrap();
        let bin = fx.home().join("trap-bin");
        fs::create_dir(&bin).unwrap();
        for name in TRAPS {
            let path = bin.join(name);
            fs::write(
                &path,
                format!("#!/bin/sh\ntouch \"$HOME/trap-{name}\"\nexit 1\n"),
            )
            .unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fx
    }

    fn home(&self) -> &Path {
        self.home.path()
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut cmd = Command::new(CCX);
        cmd.arg("manage")
            .arg("spend")
            .args(args)
            .env_clear()
            .env("HOME", self.home())
            .env("TZ", "UTC")
            .env("PATH", self.home().join("trap-bin"));
        cmd.output().expect("run ccx")
    }

    /// The trap commands that ran.
    fn traps(&self) -> Vec<String> {
        fs::read_dir(self.home())
            .unwrap()
            .filter_map(|e| {
                let name = e.unwrap().file_name().to_string_lossy().into_owned();
                name.starts_with("trap-").then_some(name)
            })
            .filter(|name| name != "trap-bin")
            .collect()
    }
}

fn testdata(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(rel)
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "ccx failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// Compares `actual` with the golden file. `CCX_UPDATE_GOLDEN=1` writes it.
fn golden(rel: &str, actual: &str) {
    let path = testdata(rel);
    if std::env::var_os("CCX_UPDATE_GOLDEN").is_some() {
        fs::write(&path, actual).unwrap();
    }
    let want = fs::read_to_string(&path).unwrap();
    assert_eq!(actual, want, "golden file {rel} is different");
}

#[test]
fn text_report_matches_the_golden_file() {
    let fx = Fixture::new();
    let out = stdout(&fx.run(&["--month", "2026-10"]));
    golden("spend/report.txt", &out);
    assert!(out.contains("estimate from local transcripts"), "{out}");
    assert!(out.contains("unpriced"), "{out}");
    assert!(
        out.contains("Skipped 2 malformed or partial transcript lines."),
        "{out}"
    );
    // Only API-key accounts are in the report.
    assert!(!out.contains("main"), "{out}");
    assert!(fx.traps().is_empty(), "{:?}", fx.traps());
}

#[test]
fn json_report_matches_the_golden_file() {
    let fx = Fixture::new();
    let out = stdout(&fx.run(&["--month", "2026-10", "--json"]));
    golden("spend/report.json", &out);
    let value: Value = serde_json::from_str(&out).unwrap();
    let names: Vec<_> = value["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["paid", "shell"]);
    assert_eq!(value["estimate"], true);
    assert!(fx.traps().is_empty(), "{:?}", fx.traps());
}

#[test]
fn one_account_by_name() {
    let fx = Fixture::new();
    let value: Value = serde_json::from_str(&stdout(
        &fx.run(&["@shell", "--month", "2026-10", "--json"]),
    ))
    .unwrap();
    assert_eq!(value["accounts"].as_array().unwrap().len(), 1);
    assert_eq!(value["accounts"][0]["name"], "shell");
    assert_eq!(value["total_usd"], 0.0);
    // The name also works with no `@`.
    let value: Value =
        serde_json::from_str(&stdout(&fx.run(&["paid", "--month", "2026-10", "--json"]))).unwrap();
    assert_eq!(value["accounts"][0]["name"], "paid");
    assert!(fx.traps().is_empty(), "{:?}", fx.traps());
}

#[test]
fn a_month_with_no_messages_has_no_rows() {
    let fx = Fixture::new();
    let value: Value =
        serde_json::from_str(&stdout(&fx.run(&["--month", "2026-08", "--json"]))).unwrap();
    assert_eq!(value["month"], "2026-08");
    assert_eq!(value["total_usd"], 0.0);
    assert!(
        value["accounts"][0]["models"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn the_default_month_is_the_current_month() {
    let fx = Fixture::new();
    let value: Value = serde_json::from_str(&stdout(&fx.run(&["--json"]))).unwrap();
    let month = value["month"].as_str().unwrap();
    assert_eq!(month.len(), 7, "{month}");
    assert_eq!(&month[4..5], "-");
}

#[test]
fn a_subscription_or_unknown_account_is_an_error() {
    let fx = Fixture::new();
    let out = fx.run(&["@main"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("`main` is not an API-key account"), "{err}");
    let out = fx.run(&["@nobody"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown account `nobody`"));
}

#[test]
fn a_bad_month_is_an_error() {
    let fx = Fixture::new();
    let out = fx.run(&["--month", "2026-13"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("YYYY-MM"));
}

/// Guard: the spend module never reads a key, never runs a command and
/// never opens a network connection.
#[test]
fn spend_source_never_reads_keys_or_uses_the_network() {
    let text =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/spend.rs")).unwrap();
    let code = text.split("#[cfg(test)]").next().unwrap();
    for forbidden in [
        "keystore",
        "KeySource",
        "key_command",
        "Command",
        "std::net",
        "TcpStream",
        "UdpSocket",
        "credentials",
    ] {
        assert!(
            !code.contains(forbidden),
            "src/spend.rs contains `{forbidden}`"
        );
    }
}

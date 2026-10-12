//! Tests for `scripts/check-emails.sh`. Each test makes a temp git repo.
//! The bad address is built at runtime, so no real-looking address is in
//! this file.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const GOOD: &str = "someone@example.com";
const NOREPLY: &str = "1+someone@users.noreply.github.com";

fn bad_address() -> String {
    ["someone", "example.org"].join("@")
}

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/check-emails.sh")
}

/// Runs git in `dir` with no user or system config.
fn git(dir: &Path, args: &[&str], author: &str, committer: &str) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("HOME", dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", author)
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", committer)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

/// Makes a repo with one commit that tracks `contents` in `notes.txt`.
fn repo(contents: &str, author: &str, committer: &str, message: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    git(dir.path(), &["init", "-q"], GOOD, GOOD);
    std::fs::write(dir.path().join("notes.txt"), contents).expect("write file");
    git(dir.path(), &["add", "notes.txt"], GOOD, GOOD);
    git(
        dir.path(),
        &["commit", "-q", "--no-gpg-sign", "-m", message],
        author,
        committer,
    );
    dir
}

fn check(dir: &Path) -> Output {
    Command::new(script())
        .arg(dir)
        .env("HOME", dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("run check-emails.sh")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn passes_on_clean_repo() {
    let contents = format!("Write to {GOOD}.\nThe bot is {NOREPLY}.\n");
    let dir = repo(&contents, NOREPLY, GOOD, "chore: add notes");
    let out = check(dir.path());
    assert!(out.status.success(), "stdout: {}", stdout(&out));
}

#[test]
fn fails_on_address_in_tracked_file() {
    let contents = format!("line one\nWrite to {}.\n", bad_address());
    let dir = repo(&contents, GOOD, GOOD, "chore: add notes");
    let out = check(dir.path());
    assert!(!out.status.success());
    assert!(
        stdout(&out).contains("notes.txt:2: Write to"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn fails_on_commit_author() {
    let dir = repo("text\n", &bad_address(), GOOD, "chore: add notes");
    let out = check(dir.path());
    assert!(!out.status.success());
    assert!(stdout(&out).contains(" author: "), "{}", stdout(&out));
}

#[test]
fn fails_on_commit_committer() {
    let dir = repo("text\n", GOOD, &bad_address(), "chore: add notes");
    let out = check(dir.path());
    assert!(!out.status.success());
    assert!(stdout(&out).contains(" committer: "), "{}", stdout(&out));
}

#[test]
fn fails_on_commit_message() {
    let message = format!("chore: add notes\n\nAsk {} for help.", bad_address());
    let dir = repo("text\n", GOOD, GOOD, &message);
    let out = check(dir.path());
    assert!(!out.status.success());
    assert!(stdout(&out).contains(" message:3: Ask"), "{}", stdout(&out));
}

#[test]
fn ignores_untracked_files() {
    let dir = repo("text\n", GOOD, GOOD, "chore: add notes");
    std::fs::write(dir.path().join("config.local.toml"), bad_address()).expect("write file");
    let out = check(dir.path());
    assert!(out.status.success(), "stdout: {}", stdout(&out));
}

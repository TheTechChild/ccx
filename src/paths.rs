//! Canonical account paths.
//!
//! Claude Code keys an account by the exact text of its config directory.
//! `canonical` must give the same text as Claude Code, or one account
//! splits into two identities. It never resolves symlinks.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

/// Makes the canonical form of `path`: expand a leading `~` to `home`,
/// clean the path lexically, remove a trailing slash and NFC-normalize.
pub fn canonical(path: &str, home: &OsStr) -> PathBuf {
    let expanded = expand_tilde(path, home);
    let cleaned = clean(&expanded);
    match cleaned.to_str() {
        Some(text) => PathBuf::from(text.nfc().collect::<String>()),
        // A path that is not UTF-8 cannot be normalized. Keep it as it is.
        None => cleaned,
    }
}

fn expand_tilde(path: &str, home: &OsStr) -> PathBuf {
    if path == "~" {
        PathBuf::from(home)
    } else if let Some(rest) = path.strip_prefix("~/") {
        Path::new(home).join(rest)
    } else {
        PathBuf::from(path)
    }
}

/// Cleans a path like Go's `filepath.Clean`: no `.` parts, no repeated
/// or trailing slashes, and `..` removes the part before it.
fn clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    let mut depth = 0usize; // Normal parts that a `..` can remove.
    for part in path.components() {
        match part {
            Component::Prefix(_) | Component::RootDir => out.push(part),
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => {
                out.pop();
                depth -= 1;
            }
            Component::ParentDir if out.has_root() => {}
            Component::ParentDir => out.push(".."),
            Component::Normal(name) => {
                out.push(name);
                depth += 1;
            }
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/test";

    fn canon(path: &str) -> PathBuf {
        canonical(path, OsStr::new(HOME))
    }

    #[test]
    fn golden_paths() {
        let cases = [
            ("~/.claude-angel2", "/home/test/.claude-angel2"),
            ("~/.claude-angel2/", "/home/test/.claude-angel2"),
            ("~/.claude-angel2//", "/home/test/.claude-angel2"),
            ("~", "/home/test"),
            ("~/", "/home/test"),
            ("~/.claude", "/home/test/.claude"),
            ("/a/./b/../c//d/", "/a/c/d"),
            ("/../a", "/a"),
            ("/", "/"),
            ("relative/dir/", "relative/dir"),
            ("../x", "../x"),
            ("~other/x", "~other/x"),
        ];
        for (input, want) in cases {
            assert_eq!(canon(input), PathBuf::from(want), "input {input:?}");
        }
    }

    #[test]
    fn non_nfc_path_is_normalized() {
        // "e" followed by a combining acute accent becomes one "é".
        let decomposed = "~/.claude-cafe\u{301}";
        assert_eq!(
            canon(decomposed),
            PathBuf::from("/home/test/.claude-caf\u{e9}")
        );
    }

    #[test]
    fn symlinks_are_not_resolved() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("link");
        std::fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let home = dir.path().as_os_str();
        assert_eq!(canonical("~/link/", home), link);
        assert_eq!(canonical("~/link/../link", home), link);
    }
}

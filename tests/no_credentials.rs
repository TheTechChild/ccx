//! Guard test for ADR 0002: ccx never reads, keeps or refreshes Claude
//! Code credentials. The test scans the non-test code in `src/` (the text
//! before the first `#[cfg(test)]` of each file).

use std::fs;
use std::path::Path;

/// Text that only code which refreshes or reads a credential needs.
const FORBIDDEN: [&str; 6] = [
    // The OAuth token endpoint.
    "oauth/token",
    // macOS keychain commands that print a secret value.
    "find-generic-password",
    "find-internet-password",
    "dump-keychain -d",
    "\"-d\"",
    // A crate that reads the OS secret store.
    "keyring::",
];

/// The credentials file. Only the sync never-share list can name it.
const CREDENTIALS_FILE: &str = ".credentials.json";
const CREDENTIALS_ALLOWED_IN: &str = "sync.rs";

/// The problems in the non-test part of one source file.
fn problems(file_name: &str, text: &str) -> Vec<String> {
    let code = text.split("#[cfg(test)]").next().unwrap_or("");
    let mut out = Vec::new();
    for (n, line) in code.lines().enumerate() {
        let where_ = format!("{file_name}:{}", n + 1);
        for pattern in FORBIDDEN {
            if line.contains(pattern) {
                out.push(format!("{where_}: contains `{pattern}`"));
            }
        }
        if line.contains(CREDENTIALS_FILE) && file_name != CREDENTIALS_ALLOWED_IN {
            out.push(format!("{where_}: names `{CREDENTIALS_FILE}`"));
        }
    }
    out
}

#[test]
fn source_never_touches_credentials() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    let mut files = 0;
    for entry in fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            files += 1;
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            found.extend(problems(&name, &fs::read_to_string(&path).unwrap()));
        }
    }
    assert!(files > 5, "the scan found only {files} files");
    assert!(
        found.is_empty(),
        "credential access in src/:\n{}",
        found.join("\n")
    );
}

#[test]
fn sync_names_the_credentials_file_only_to_never_share_it() {
    let text =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sync.rs")).unwrap();
    let code = text.split("#[cfg(test)]").next().unwrap();
    for line in code.lines().filter(|l| l.contains(CREDENTIALS_FILE)) {
        // The only use is a string item in a list, with no file call.
        assert_eq!(line.trim(), format!("\"{CREDENTIALS_FILE}\","), "{line}");
    }
}

/// The copy of MCP servers and preferences (#17) reads only `.claude.json`
/// and `sessions/<pid>.json`. It never names a credential file, the MCP
/// auth cache, a `.key` file or the keychain.
#[test]
fn prefs_names_no_credential_store() {
    let text =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/prefs.rs")).unwrap();
    let code = text.split("#[cfg(test)]").next().unwrap();
    for pattern in [
        "credentials",
        "mcp-needs-auth",
        ".key\"",
        "security",
        "keychain",
        "accessToken",
        "refreshToken",
    ] {
        assert!(!code.contains(pattern), "src/prefs.rs contains `{pattern}`");
    }
}

#[test]
fn guard_finds_each_forbidden_pattern() {
    let samples = [
        "let url = \"https://example.com/v1/oauth/token\";",
        "Command::new(\"security\").args([\"find-generic-password\", \"-w\"])",
        "Command::new(\"security\").arg(\"find-internet-password\")",
        "sh -c 'security dump-keychain -d'",
        "Command::new(\"security\").args([\"dump-keychain\", \"-d\"])",
        "let entry = keyring::Entry::new(\"Claude Code-credentials\", \"x\");",
        "fs::read(dir.join(\".credentials.json\"))",
    ];
    for sample in samples {
        assert!(
            !problems("launch.rs", sample).is_empty(),
            "missed: {sample}"
        );
    }
    // Test code after `#[cfg(test)]` is not scanned.
    assert!(problems("launch.rs", "ok\n#[cfg(test)]\noauth/token\n").is_empty());
    // The attribute-only keychain listing of doctor is allowed.
    assert!(
        problems(
            "doctor.rs",
            "Command::new(\"security\").arg(\"dump-keychain\")"
        )
        .is_empty()
    );
}

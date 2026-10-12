//! The launch path: `ccx [@name] [claude args...]`.
//!
//! A claude subcommand (`ccx [@name] auth ...`) uses the same path, but
//! ccx never selects its account by usage.
//!
//! Keep this path fast. It reads the config, builds the environment, finds
//! `claude` and replaces the ccx process with it.

use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::config::{Account, Config, Result};
use crate::env::Env;

/// The first words that ccx keeps for itself.
pub const RESERVED: [&str; 4] = ["manage", "status", "usage", "handoff"];

/// Claude Code subcommands (with aliases) that change one account. ccx never
/// picks an account for them by usage: they run in `@name`, in
/// `CCX_ACCOUNT`, or in the base account.
///
/// Source: the `Commands:` section of `claude --help`, Claude Code 2.1.296.
/// To update: compare this list with the `Commands:` section of
/// `claude --help` for the new version, then change the version above.
/// `config` is not a claude subcommand. Do not add it.
pub const CLAUDE_SUBCOMMANDS: [&str; 21] = [
    "agents",
    "attach",
    "auth",
    "auto-mode",
    "doctor",
    "gateway",
    "import",
    "install",
    "logs",
    "mcp",
    "plugin",
    "plugins",
    "purge",
    "respawn",
    "rm",
    "setup-token",
    "stop",
    "kill",
    "ultrareview",
    "update",
    "upgrade",
];

/// Variables that override the account login. ccx removes them.
pub const STRIPPED: [&str; 4] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_SECURESTORAGE_CONFIG_DIR",
];

/// ccx refuses to start when `CCX_DEPTH` is this value or more. Each launch
/// adds one, so a `claude` that runs ccx again cannot loop forever.
pub const MAX_DEPTH: u32 = 5;

/// Places to look for `claude` after PATH. A leading `~` is `$HOME`.
const FALLBACK_DIRS: [&str; 3] = ["~/.local/bin", "/opt/homebrew/bin", "/usr/local/bin"];

#[derive(Debug, PartialEq, Eq)]
pub enum Invocation {
    /// A ccx command: a reserved first word, or `--version` alone.
    Own,
    /// Launch claude. `account` is the name after `@`, if there is one.
    /// `subcommand` is the claude subcommand in the first claude argument.
    Launch {
        account: Option<String>,
        args: Vec<OsString>,
        subcommand: Option<&'static str>,
    },
}

impl Invocation {
    fn launch(account: Option<String>, args: Vec<OsString>) -> Self {
        let subcommand = claude_subcommand(&args);
        Invocation::Launch {
            account,
            args,
            subcommand,
        }
    }
}

/// The claude subcommand when it is the first claude argument.
pub fn claude_subcommand(args: &[OsString]) -> Option<&'static str> {
    let first = args.first()?.to_str()?;
    CLAUDE_SUBCOMMANDS.into_iter().find(|word| *word == first)
}

/// Splits the command line (without the program name).
pub fn parse_args(mut args: Vec<OsString>) -> Result<Invocation> {
    let Some(first) = args.first().and_then(|a| a.to_str()) else {
        return Ok(Invocation::launch(None, args));
    };
    if first == "--" {
        args.remove(0);
        return Ok(Invocation::launch(None, args));
    }
    if RESERVED.contains(&first) || (args.len() == 1 && matches!(first, "--version" | "-V")) {
        return Ok(Invocation::Own);
    }
    if let Some(name) = first.strip_prefix('@') {
        if name.is_empty() {
            return Err("`@` needs an account name, for example `@main`".into());
        }
        let account = Some(name.to_string());
        args.remove(0);
        return Ok(Invocation::launch(account, args));
    }
    Ok(Invocation::launch(None, args))
}

/// How ccx gets the account for a launch.
#[derive(Debug, PartialEq)]
pub enum Pick<'a> {
    /// `@name` or `$CCX_ACCOUNT` names the account.
    Named(&'a Account),
    /// A claude subcommand with no name: always the base account.
    Base(&'a Account),
    /// A session with no name: automatic selection. Only this variant can
    /// start a usage fetch or the picker.
    Auto,
}

/// Finds the account: `@name`, else `$CCX_ACCOUNT`, else the base account
/// for a claude subcommand, else automatic selection. ccx never reads
/// `CCX_ACTIVE` here.
pub fn pick_account<'a>(
    config: &'a Config,
    name: Option<&str>,
    subcommand: Option<&str>,
    env: &Env,
) -> Result<Pick<'a>> {
    let name = match (name, env.get("CCX_ACCOUNT")) {
        (Some(name), _) => name.to_string(),
        (None, Some(value)) => value.to_string_lossy().into_owned(),
        (None, None) if subcommand.is_some() => return Ok(Pick::Base(config.base())),
        (None, None) => return Ok(Pick::Auto),
    };
    let account = config.account(&name).ok_or_else(|| {
        format!(
            "unknown account `{name}`; known accounts: {}",
            config.names().join(", ")
        )
    })?;
    Ok(Pick::Named(account))
}

/// The note that a claude subcommand with no name runs on the base account.
pub fn base_note(subcommand: &str, base: &Account) -> String {
    format!(
        "ccx: '{subcommand}' runs on base account {}; use @name for another account",
        base.name
    )
}

/// Reads `CCX_DEPTH` and fails at the limit. Returns the current depth.
pub fn check_depth(env: &Env) -> Result<u32> {
    let depth = env
        .get("CCX_DEPTH")
        .and_then(|v| v.to_str())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if depth >= MAX_DEPTH {
        return Err(format!(
            "CCX_DEPTH is {depth}: ccx started itself {MAX_DEPTH} times. \
             `claude` probably runs ccx again. Set launch.claude_bin or \
             CCX_CLAUDE_BIN to the real claude binary"
        ));
    }
    Ok(depth)
}

/// Makes the environment for claude. Returns it and the names of the
/// variables that ccx removed.
pub fn build_env(parent: &Env, account: &Account, depth: u32) -> (Env, Vec<&'static str>) {
    let mut env = parent.clone();
    let removed = STRIPPED
        .into_iter()
        .filter(|key| env.remove(key).is_some())
        .collect();
    if account.is_base {
        env.remove("CLAUDE_CONFIG_DIR");
    } else {
        env.set("CLAUDE_CONFIG_DIR", account.dir.as_os_str());
    }
    env.set("CCX_ACTIVE", account.name.as_str());
    env.set("CCX_DEPTH", (depth + 1).to_string());
    (env, removed)
}

/// Finds `claude`: `launch.claude_bin`, `$CCX_CLAUDE_BIN`, PATH, then the
/// fallback dirs. The search skips `self_exe` (ccx itself).
pub fn find_claude(config: &Config, env: &Env, self_exe: Option<&Path>) -> Option<PathBuf> {
    if let Some(bin) = &config.launch.claude_bin {
        return Some(bin.clone());
    }
    if let Some(bin) = env.get("CCX_CLAUDE_BIN") {
        return Some(PathBuf::from(bin));
    }
    let home = env.get("HOME").unwrap_or(OsStr::new("/"));
    let path_dirs = env
        .get("PATH")
        .map(|p| std::env::split_paths(p).collect::<Vec<_>>())
        .unwrap_or_default();
    let fallback_dirs = FALLBACK_DIRS
        .iter()
        .map(|dir| crate::paths::canonical(dir, home));
    path_dirs
        .into_iter()
        .chain(fallback_dirs)
        .map(|dir| dir.join("claude"))
        .find(|candidate| is_executable(candidate) && !same_file(candidate, self_exe))
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// True when `path` is the same file as `other` after symlinks.
fn same_file(path: &Path, other: Option<&Path>) -> bool {
    let (Some(other), Ok(a)) = (other, std::fs::metadata(path)) else {
        return false;
    };
    std::fs::metadata(other).is_ok_and(|b| a.dev() == b.dev() && a.ino() == b.ino())
}

/// The text that `CCX_DRY_RUN=1` prints.
pub fn dry_run_report(
    account: &Account,
    claude: Option<&Path>,
    parent: &Env,
    env: &Env,
    args: &[OsString],
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "account: {}", account.name);
    let _ = writeln!(out, "dir: {}", account.dir.display());
    let claude = claude.map_or("not found".into(), |p| p.display().to_string());
    let _ = writeln!(out, "claude: {claude}");
    out.push_str("env:\n");
    for line in env_diff(parent, env) {
        let _ = writeln!(out, "  {line}");
    }
    let argv: Vec<_> = args.iter().map(|a| a.to_string_lossy()).collect();
    let _ = writeln!(out, "argv: {argv:?}");
    out
}

/// Lines for each changed variable: `- NAME` (removed), `+ NAME=value`
/// (added) or `~ NAME=value` (changed). Removed values are not shown,
/// because they can be secrets.
fn env_diff(before: &Env, after: &Env) -> Vec<String> {
    let mut lines = Vec::new();
    for (key, _) in before.iter() {
        if !after.contains(&key.to_string_lossy()) {
            lines.push(format!("- {}", key.to_string_lossy()));
        }
    }
    let old: std::collections::BTreeMap<_, _> = before.iter().collect();
    for (key, value) in after.iter() {
        let mark = match old.get(key) {
            None => '+',
            Some(old) if *old != value => '~',
            Some(_) => continue,
        };
        lines.push(format!(
            "{mark} {}={}",
            key.to_string_lossy(),
            value.to_string_lossy()
        ));
    }
    lines.sort_by(|a, b| a[2..].cmp(&b[2..]));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Source, parse};

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn config() -> Config {
        let text =
            "[accounts.main]\ndir = \"~/.claude\"\n[accounts.angel2]\ndir = \"~/.claude-angel2\"\n";
        let main = Source {
            path: Path::new("config.toml"),
            text,
        };
        parse(&main, None, OsStr::new("/home/test")).unwrap()
    }

    fn launch(account: Option<&str>, args: &[&str]) -> Invocation {
        Invocation::Launch {
            account: account.map(String::from),
            args: os(args),
            subcommand: None,
        }
    }

    fn routed(account: Option<&str>, args: &[&str], word: &'static str) -> Invocation {
        Invocation::Launch {
            account: account.map(String::from),
            args: os(args),
            subcommand: Some(word),
        }
    }

    #[test]
    fn subcommand_list_has_21_unique_words_and_no_config() {
        let mut words = CLAUDE_SUBCOMMANDS.to_vec();
        words.sort_unstable();
        words.dedup();
        assert_eq!(words.len(), 21);
        assert!(!CLAUDE_SUBCOMMANDS.contains(&"config"));
        for word in CLAUDE_SUBCOMMANDS {
            assert!(!RESERVED.contains(&word), "{word} is a ccx word");
        }
    }

    #[test]
    fn each_subcommand_is_found_only_as_the_first_claude_argument() {
        for word in CLAUDE_SUBCOMMANDS {
            assert_eq!(
                parse_args(os(&[word, "x"])).unwrap(),
                routed(None, &[word, "x"], word)
            );
            assert_eq!(
                parse_args(os(&["@angel2", word])).unwrap(),
                routed(Some("angel2"), &[word], word)
            );
            assert_eq!(
                parse_args(os(&["--", word, "x"])).unwrap(),
                routed(None, &[word, "x"], word)
            );
            for args in [&["-p", word][..], &["--model", "opus", word], &["x", word]] {
                assert_eq!(parse_args(os(args)).unwrap(), launch(None, args));
            }
            assert_eq!(
                parse_args(os(&["@angel2", "-p", word])).unwrap(),
                launch(Some("angel2"), &["-p", word])
            );
        }
        // Case and prefixes do not match.
        for args in [&["Auth"][..], &["auth-x"], &["--auth"]] {
            assert_eq!(parse_args(os(args)).unwrap(), launch(None, args));
        }
    }

    #[test]
    fn config_goes_to_the_normal_launch_path() {
        assert_eq!(
            parse_args(os(&["config"])).unwrap(),
            launch(None, &["config"])
        );
        let config = config();
        let pick = pick_account(
            &config,
            None,
            claude_subcommand(&os(&["config"])),
            &Env::default(),
        );
        assert_eq!(pick.unwrap(), Pick::Auto);
    }

    #[test]
    fn parse_args_cases() {
        let cases: [(&[&str], Invocation); 10] = [
            (&[], launch(None, &[])),
            (&["-p", "hi"], launch(None, &["-p", "hi"])),
            (
                &["@angel2", "--model", "opus"],
                launch(Some("angel2"), &["--model", "opus"]),
            ),
            (&["@angel2", "status"], launch(Some("angel2"), &["status"])),
            (
                &["@angel2", "--", "x"],
                launch(Some("angel2"), &["--", "x"]),
            ),
            (&["--", "status"], launch(None, &["status"])),
            (&["--", "@main"], launch(None, &["@main"])),
            (&["status"], Invocation::Own),
            (&["--version"], Invocation::Own),
            (&["--version", "x"], launch(None, &["--version", "x"])),
        ];
        for (args, want) in cases {
            assert_eq!(parse_args(os(args)).unwrap(), want, "{args:?}");
        }
        for word in RESERVED {
            assert_eq!(parse_args(os(&[word, "x"])).unwrap(), Invocation::Own);
        }
        assert!(parse_args(os(&["@"])).is_err());
    }

    #[test]
    fn pick_account_order() {
        let config = config();
        let main = config.account("main").unwrap();
        let angel2 = config.account("angel2").unwrap();
        let none = Env::default();
        let forced: Env = [("CCX_ACCOUNT", "angel2")].into_iter().collect();
        let active: Env = [("CCX_ACTIVE", "angel2")].into_iter().collect();
        let pick = |name, sub, env| pick_account(&config, name, sub, env).unwrap();

        assert_eq!(pick(None, None, &none), Pick::Auto);
        assert_eq!(pick(None, None, &active), Pick::Auto);
        assert_eq!(pick(None, None, &forced), Pick::Named(angel2));
        assert_eq!(pick(Some("main"), None, &forced), Pick::Named(main));
        assert_eq!(pick(None, Some("auth"), &none), Pick::Base(main));
        assert_eq!(pick(None, Some("auth"), &active), Pick::Base(main));
        assert_eq!(pick(None, Some("plugin"), &forced), Pick::Named(angel2));
        assert_eq!(
            pick(Some("angel2"), Some("mcp"), &none),
            Pick::Named(angel2)
        );
        assert_eq!(pick(Some("main"), Some("mcp"), &forced), Pick::Named(main));

        let err = pick_account(&config, Some("nope"), None, &none).unwrap_err();
        assert_eq!(err, "unknown account `nope`; known accounts: angel2, main");
        let err = pick_account(&config, Some("nope"), Some("auth"), &none).unwrap_err();
        assert!(err.contains("unknown account `nope`"));
    }

    /// `Pick::Auto` is the only way to the usage fetch and the picker.
    /// A claude subcommand never gets it, with or without a name.
    #[test]
    fn subcommands_never_reach_automatic_selection() {
        let config = config();
        let envs: [Env; 3] = [
            Env::default(),
            [("CCX_ACCOUNT", "angel2")].into_iter().collect(),
            [("CCX_ACTIVE", "angel2")].into_iter().collect(),
        ];
        for word in CLAUDE_SUBCOMMANDS {
            for args in [&[word, "x"][..], &["--", word], &["@angel2", word]] {
                let Invocation::Launch {
                    account,
                    subcommand,
                    ..
                } = parse_args(os(args)).unwrap()
                else {
                    panic!("{args:?} is not a launch");
                };
                assert_eq!(subcommand, Some(word));
                for env in &envs {
                    let pick = pick_account(&config, account.as_deref(), subcommand, env);
                    assert_ne!(pick.unwrap(), Pick::Auto, "{args:?}");
                }
            }
        }
    }

    #[test]
    fn base_note_text() {
        let config = config();
        assert_eq!(
            base_note("auth", config.base()),
            "ccx: 'auth' runs on base account main; use @name for another account"
        );
    }

    #[test]
    fn build_env_for_other_and_base_accounts() {
        let config = config();
        let parent: Env = [
            ("ANTHROPIC_API_KEY", "k"),
            ("CLAUDE_CODE_OAUTH_TOKEN", ""),
            ("CLAUDE_CONFIG_DIR", "/elsewhere"),
            ("KEEP", "1"),
        ]
        .into_iter()
        .collect();

        let (env, removed) = build_env(&parent, config.account("angel2").unwrap(), 0);
        assert_eq!(removed, ["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN"]);
        assert!(!env.contains("ANTHROPIC_API_KEY"));
        assert!(!env.contains("CLAUDE_CODE_OAUTH_TOKEN"));
        assert_eq!(
            env.get("CLAUDE_CONFIG_DIR"),
            Some(OsStr::new("/home/test/.claude-angel2"))
        );
        assert_eq!(env.get("CCX_ACTIVE"), Some(OsStr::new("angel2")));
        assert_eq!(env.get("CCX_DEPTH"), Some(OsStr::new("1")));
        assert_eq!(env.get("KEEP"), Some(OsStr::new("1")));

        let (env, _) = build_env(&parent, config.base(), 2);
        assert!(!env.contains("CLAUDE_CONFIG_DIR"));
        assert_eq!(env.get("CCX_ACTIVE"), Some(OsStr::new("main")));
        assert_eq!(env.get("CCX_DEPTH"), Some(OsStr::new("3")));
    }

    #[test]
    fn depth_limit() {
        let at = |v: &str| -> Env { [("CCX_DEPTH", v)].into_iter().collect() };
        assert_eq!(check_depth(&Env::default()).unwrap(), 0);
        assert_eq!(check_depth(&at("junk")).unwrap(), 0);
        assert_eq!(check_depth(&at("4")).unwrap(), 4);
        assert!(
            check_depth(&at("5"))
                .unwrap_err()
                .contains("CCX_DEPTH is 5")
        );
    }

    #[test]
    fn find_claude_order_and_self_skip() {
        let dir = tempfile::tempdir().unwrap();
        let make = |name: &str, mode: u32| {
            let sub = dir.path().join(name);
            std::fs::create_dir_all(&sub).unwrap();
            let file = sub.join("claude");
            std::fs::write(&file, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
            file
        };
        let me = make("self", 0o755);
        let plain = make("plain", 0o644);
        let real = make("real", 0o755);
        let link_dir = dir.path().join("link");
        std::fs::create_dir(&link_dir).unwrap();
        std::os::unix::fs::symlink(&me, link_dir.join("claude")).unwrap();

        let path = std::env::join_paths([
            link_dir.as_path(),
            plain.parent().unwrap(),
            real.parent().unwrap(),
        ])
        .unwrap();
        let mut env: Env = [("PATH", path.as_os_str())].into_iter().collect();
        let mut config = config();

        // PATH: skip the link to ccx and the file that is not executable.
        assert_eq!(find_claude(&config, &env, Some(&me)), Some(real.clone()));
        // Without ccx itself known, the link is a valid claude.
        assert_eq!(
            find_claude(&config, &env, None),
            Some(link_dir.join("claude"))
        );

        env.set("CCX_CLAUDE_BIN", "/env/claude");
        assert_eq!(
            find_claude(&config, &env, Some(&me)),
            Some(PathBuf::from("/env/claude"))
        );

        config.launch.claude_bin = Some(PathBuf::from("/config/claude"));
        assert_eq!(
            find_claude(&config, &env, Some(&me)),
            Some(PathBuf::from("/config/claude"))
        );
    }

    #[test]
    fn dry_run_report_shows_account_env_diff_and_argv() {
        let config = config();
        let parent: Env = [
            ("ANTHROPIC_API_KEY", "secret-value"),
            ("CCX_ACTIVE", "main"),
            ("KEEP", "1"),
        ]
        .into_iter()
        .collect();
        let account = config.account("angel2").unwrap();
        let (env, _) = build_env(&parent, account, 0);
        let report = dry_run_report(
            account,
            Some(Path::new("/bin/claude")),
            &parent,
            &env,
            &os(&["--model", "opus"]),
        );
        let want = "\
account: angel2
dir: /home/test/.claude-angel2
claude: /bin/claude
env:
  - ANTHROPIC_API_KEY
  ~ CCX_ACTIVE=angel2
  + CCX_DEPTH=1
  + CLAUDE_CONFIG_DIR=/home/test/.claude-angel2
argv: [\"--model\", \"opus\"]
";
        assert_eq!(report, want);
        assert!(!report.contains("secret-value"));
    }
}

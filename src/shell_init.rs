//! `ccx manage shell-init zsh|bash|fish`: print shell code for the profile.
//!
//! The output has two parts: completions for `ccx` (from the clap command
//! tree, plus one `@name` token for each account) and one alias for each
//! entry in `aliases = [...]`. Bare `claude` stays the base account, so the
//! output never defines `claude`.

use std::fmt::Write as _;

use clap::{Command, ValueEnum};
use clap_complete::Shell as Generator;

use crate::config::{self, Config, Result};
use crate::env::Env;

/// The shells that shell-init supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    Zsh,
    Bash,
    Fish,
}

/// Names that an alias must never replace.
const FORBIDDEN: [&str; 2] = ["claude", "ccx"];

/// Prints the shell code for `shell`. With no `config.toml`, the output has
/// only the completions, so a new machine can have the profile line before
/// `ccx manage adopt`.
pub fn run(env: &Env, shell: Shell, cli: Command) -> Result<()> {
    let config = if config::config_path(env)?.exists() {
        Some(config::load(env)?)
    } else {
        None
    };
    print!("{}", render(shell, cli, config.as_ref())?);
    Ok(())
}

/// The full shell-init output. It checks all aliases before it makes any
/// text, so an error never gives half a script.
pub fn render(shell: Shell, cli: Command, config: Option<&Config>) -> Result<String> {
    let aliases = match config {
        Some(config) => aliases(config)?,
        None => Vec::new(),
    };
    let names: Vec<&str> = config.map(Config::names).unwrap_or_default();

    let mut out = String::new();
    let _ = writeln!(
        out,
        "# ccx shell-init ({}). Generated; do not edit.",
        shell.name()
    );
    out.push_str(&completions(shell, cli, &names));
    if !out.ends_with('\n') {
        out.push('\n');
    }
    for (alias, account) in aliases {
        let target = format!("ccx @{account}");
        let line = match shell {
            Shell::Zsh | Shell::Bash => format!("alias {alias}={}", posix_quote(&target)),
            Shell::Fish => format!("alias {alias} {}", fish_quote(&target)),
        };
        out.push_str(&line);
        out.push('\n');
    }
    Ok(out)
}

impl Shell {
    fn name(self) -> &'static str {
        match self {
            Shell::Zsh => "zsh",
            Shell::Bash => "bash",
            Shell::Fish => "fish",
        }
    }
}

/// Each (alias, account name) pair, in account name order. An alias must
/// be a valid shell identifier, must not be `claude` or `ccx`, and must be
/// unique.
pub fn aliases(config: &Config) -> Result<Vec<(&str, &str)>> {
    let mut seen: Vec<(&str, &str)> = Vec::new();
    for account in &config.accounts {
        for alias in &account.aliases {
            if !valid_identifier(alias) {
                return Err(format!(
                    "accounts.{}.aliases: `{alias}` is not a valid shell identifier \
                     (use letters, digits and `_`, and do not start with a digit)",
                    account.name
                ));
            }
            if FORBIDDEN.contains(&alias.as_str()) {
                return Err(format!(
                    "accounts.{}.aliases: ccx never makes an alias `{alias}`",
                    account.name
                ));
            }
            if let Some((_, other)) = seen.iter().find(|(a, _)| *a == alias) {
                return Err(format!(
                    "accounts.{}.aliases: account `{other}` also uses `{alias}`",
                    account.name
                ));
            }
            seen.push((alias, &account.name));
        }
    }
    Ok(seen)
}

/// A shell identifier: `^[A-Za-z_][A-Za-z0-9_]*$`.
pub fn valid_identifier(word: &str) -> bool {
    let mut chars = word.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Single quotes for sh, bash and zsh. A `'` becomes `'\''`.
pub(crate) fn posix_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Single quotes for fish. In fish, `\` and `'` need a `\` in single quotes.
fn fish_quote(text: &str) -> String {
    format!("'{}'", text.replace('\\', r"\\").replace('\'', r"\'"))
}

/// The clap completions, with one extra `@name` subcommand for each
/// account, so that the shell completes `ccx @<TAB>`.
fn completions(shell: Shell, cli: Command, names: &[&str]) -> String {
    let mut cmd = cli;
    for name in names {
        cmd = cmd.subcommand(
            Command::new(format!("@{name}"))
                .about(format!("Start claude as account {name}"))
                .disable_help_flag(true),
        );
    }
    let text = generate(shell, cmd);
    match shell {
        Shell::Zsh => zsh_guard(&text),
        _ => text,
    }
}

/// The static completion file for the release archive: the plain clap
/// output with no `@name` tokens, because the archive has no config. The
/// zsh file has no `compdef` guard, because zsh loads it from `fpath`.
/// A test writes and checks these files.
#[cfg(test)]
pub fn completion_file(shell: Shell, cli: Command) -> String {
    generate(shell, cli)
}

/// The file name of the completion file for `shell` in `completions/`.
#[cfg(test)]
pub fn completion_file_name(shell: Shell) -> &'static str {
    match shell {
        Shell::Zsh => "_ccx",
        Shell::Bash => "ccx.bash",
        Shell::Fish => "ccx.fish",
    }
}

/// The clap completion script for `cmd`.
fn generate(shell: Shell, mut cmd: Command) -> String {
    let generator = match shell {
        Shell::Zsh => Generator::Zsh,
        Shell::Bash => Generator::Bash,
        Shell::Fish => Generator::Fish,
    };
    let mut buf = Vec::new();
    clap_complete::generate(generator, &mut cmd, "ccx", &mut buf);
    String::from_utf8(buf).expect("clap_complete writes UTF-8")
}

/// The zsh completion file calls `compdef`, which exists only after
/// `compinit`. Without this guard, a profile that runs shell-init before
/// `compinit` shows an error at each start.
fn zsh_guard(text: &str) -> String {
    text.replace(
        "    compdef _ccx ccx\n",
        "    (( $+functions[compdef] )) && compdef _ccx ccx\n",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Source, parse};
    use std::ffi::OsStr;
    use std::path::Path;

    fn config(text: &str) -> Config {
        parse(
            &Source {
                path: Path::new("config.toml"),
                text,
            },
            None,
            OsStr::new("/home/test"),
        )
        .unwrap()
    }

    const BASE: &str = "[accounts.main]\ndir = \"~/.claude\"\n";

    fn with_aliases(aliases: &str) -> Config {
        config(&format!(
            "{BASE}[accounts.angel2]\ndir = \"~/.claude-angel2\"\naliases = {aliases}\n"
        ))
    }

    #[test]
    fn identifiers() {
        for good in ["claude2", "c", "_x", "Work_2"] {
            assert!(valid_identifier(good), "{good}");
        }
        for bad in ["", "2c", "a-b", "a b", "a'b", "a;b", "$x", "é", "a.b"] {
            assert!(!valid_identifier(bad), "{bad}");
        }
    }

    #[test]
    fn aliases_are_listed_in_account_order() {
        let config = with_aliases(r#"["claude2", "c2"]"#);
        assert_eq!(
            aliases(&config).unwrap(),
            [("claude2", "angel2"), ("c2", "angel2")]
        );
    }

    #[test]
    fn bad_alias_names_are_errors() {
        let err = aliases(&with_aliases(r#"["x; rm -rf ~"]"#)).unwrap_err();
        assert!(err.contains("not a valid shell identifier"), "{err}");
        let err = aliases(&with_aliases(r#"["claude"]"#)).unwrap_err();
        assert!(err.contains("never makes an alias `claude`"), "{err}");
        let err = aliases(&with_aliases(r#"["ccx"]"#)).unwrap_err();
        assert!(err.contains("`ccx`"), "{err}");
    }

    #[test]
    fn duplicate_aliases_are_errors() {
        let config = config(&format!(
            "{BASE}aliases = [\"c\"]\n[accounts.angel2]\ndir = \"~/.claude-angel2\"\naliases = [\"c\"]\n"
        ));
        let err = aliases(&config).unwrap_err();
        assert!(err.contains("also uses `c`"), "{err}");
    }

    #[test]
    fn quoting() {
        assert_eq!(posix_quote("ccx @a"), "'ccx @a'");
        assert_eq!(posix_quote("it's"), r"'it'\''s'");
        assert_eq!(fish_quote("ccx @a"), "'ccx @a'");
        assert_eq!(fish_quote(r"it's \"), r"'it\'s \\'");
    }

    #[test]
    fn alias_lines_for_each_shell() {
        let config = with_aliases(r#"["claude2"]"#);
        let cmd = || Command::new("ccx").subcommand(Command::new("manage"));
        let zsh = render(Shell::Zsh, cmd(), Some(&config)).unwrap();
        assert!(zsh.ends_with("\nalias claude2='ccx @angel2'\n"), "{zsh}");
        let bash = render(Shell::Bash, cmd(), Some(&config)).unwrap();
        assert!(bash.ends_with("\nalias claude2='ccx @angel2'\n"), "{bash}");
        let fish = render(Shell::Fish, cmd(), Some(&config)).unwrap();
        assert!(fish.ends_with("\nalias claude2 'ccx @angel2'\n"), "{fish}");
    }

    #[test]
    fn no_config_gives_completions_only() {
        let cmd = Command::new("ccx").subcommand(Command::new("manage"));
        let out = render(Shell::Bash, cmd, None).unwrap();
        assert!(out.contains("complete -F _ccx"));
        assert!(!out.contains("alias "));
    }

    #[test]
    fn zsh_guard_applies_to_the_generated_file() {
        let cmd = Command::new("ccx").subcommand(Command::new("manage"));
        let out = completions(Shell::Zsh, cmd, &[]);
        assert!(out.contains("(( $+functions[compdef] )) && compdef _ccx ccx"));
        assert!(!out.contains("\n    compdef _ccx ccx\n"));
    }
}

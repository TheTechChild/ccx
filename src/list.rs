//! `ccx manage list [--json]`: show each configured account.

use std::ffi::OsStr;

use serde_json::json;

use crate::config::{self, Config, Result};
use crate::env::Env;

pub fn run(env: &Env, as_json: bool) -> Result<()> {
    let config = config::load(env)?;
    if as_json {
        println!("{}", to_json(&config));
    } else {
        print!("{}", to_table(&config, config::home(env)?));
    }
    Ok(())
}

/// Stable JSON: the accounts in name order, and the keys of each object
/// in a fixed (alphabetical) order. A missing email is `null`.
pub fn to_json(config: &Config) -> String {
    let accounts: Vec<_> = config
        .accounts
        .iter()
        .map(|account| {
            json!({
                "name": account.name,
                "dir": account.dir.to_string_lossy(),
                "email": account.email,
                "aliases": account.aliases,
                "base": account.is_base,
            })
        })
        .collect();
    serde_json::to_string_pretty(&json!({ "accounts": accounts }))
        .expect("JSON values always serialize")
}

/// A table with the columns NAME, DIR, EMAIL and ALIASES. A dir in HOME
/// starts with `~`. An empty value is `-`.
pub fn to_table(config: &Config, home: &OsStr) -> String {
    let mut rows = vec![["NAME", "DIR", "EMAIL", "ALIASES"].map(String::from)];
    for account in &config.accounts {
        let dir = crate::paths::tilde(&account.dir, home);
        let aliases = match account.aliases.join(", ") {
            text if text.is_empty() => "-".to_string(),
            text => text,
        };
        rows.push([
            account.name.clone(),
            dir,
            account.email.clone().unwrap_or_else(|| "-".into()),
            aliases,
        ]);
    }
    let mut widths = [0; 4];
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let mut out = String::new();
    for row in &rows {
        let line: Vec<String> = row
            .iter()
            .zip(widths)
            .map(|(cell, width)| format!("{cell:width$}"))
            .collect();
        out.push_str(line.join("  ").trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Source, parse};
    use std::path::Path;

    fn config() -> Config {
        let main = r#"
            [accounts.main]
            dir = "~/.claude"
            [accounts.angel2]
            dir = "~/.claude-angel2"
            aliases = ["claude2", "c2"]
            [accounts.other]
            dir = "/srv/claude"
        "#;
        let overlay = "[accounts.angel2]\nemail = \"work@example.com\"\n";
        parse(
            &Source {
                path: Path::new("config.toml"),
                text: main,
            },
            Some(&Source {
                path: Path::new("config.local.toml"),
                text: overlay,
            }),
            OsStr::new("/home/test"),
        )
        .unwrap()
    }

    #[test]
    fn table_aligns_columns() {
        assert_eq!(
            to_table(&config(), OsStr::new("/home/test")),
            "\
NAME    DIR               EMAIL             ALIASES
angel2  ~/.claude-angel2  work@example.com  claude2, c2
main    ~/.claude         -                 -
other   /srv/claude       -                 -
"
        );
    }

    #[test]
    fn json_is_stable() {
        let value: serde_json::Value = serde_json::from_str(&to_json(&config())).unwrap();
        assert_eq!(
            value,
            json!({"accounts": [
                {"name": "angel2", "dir": "/home/test/.claude-angel2", "email": "work@example.com",
                 "aliases": ["claude2", "c2"], "base": false},
                {"name": "main", "dir": "/home/test/.claude", "email": null, "aliases": [], "base": true},
                {"name": "other", "dir": "/srv/claude", "email": null, "aliases": [], "base": false},
            ]})
        );
        // Same input, same text.
        assert_eq!(to_json(&config()), to_json(&config()));
    }
}

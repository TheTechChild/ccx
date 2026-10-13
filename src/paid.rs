//! API-key accounts as a paid fallback (#20): the consent rules and the
//! key in the launch environment.
//!
//! ccx asks before it uses a paid account. With no terminal, ccx uses a
//! paid account only when the user gives `--allow-paid`.

use std::io::IsTerminal;

use crate::config::{Account, Result};
use crate::env::Env;
use crate::keystore::{self, SecretStore};

/// The variable that gives the key to claude. ccx sets it only for the
/// one exec of a paid launch.
pub const KEY_VAR: &str = "ANTHROPIC_API_KEY";

/// The kind text of an API-key account in `ccx status` and `ccx manage
/// list`.
pub const KIND_LABEL: &str = "api-key (paid)";

/// True when ccx can use a paid account. `--allow-paid` (`allow`) says
/// yes with no prompt. With no terminal, the answer is no. Else `ask` gets
/// the answer of the user.
pub fn consent(allow: bool, terminal: bool, ask: impl FnOnce() -> Result<bool>) -> Result<bool> {
    if allow {
        return Ok(true);
    }
    if !terminal {
        return Ok(false);
    }
    ask()
}

/// The question when all subscription accounts are limited.
pub fn fallback_question(name: &str) -> String {
    format!("All subscription accounts are limited. Use paid account {name}?")
}

/// The question for `ccx @name` on an API-key account.
pub fn named_question(name: &str) -> String {
    format!("Account {name} is a paid API-key account. Use it?")
}

/// The error when ccx does not use a paid account that the user named.
pub fn refused(name: &str) -> String {
    format!(
        "account {name} is a paid API-key account; ccx did not start claude. To use it, run `ccx --allow-paid @{name}` or answer yes on a terminal"
    )
}

/// True when ccx can ask: stdin and stderr are terminals.
pub fn terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// Asks a yes/no question on the terminal. The default is no.
pub fn ask(question: &str) -> Result<bool> {
    dialoguer::Confirm::new()
        .with_prompt(question)
        .default(false)
        .interact()
        .map_err(|e| format!("cannot read the answer: {e}"))
}

/// Asks before a launch on the paid account `account`, for `ccx @name`
/// and a resume. Returns an error when the answer is no.
pub fn confirm_named(account: &Account, allow: bool) -> Result<()> {
    if consent(allow, terminal(), || ask(&named_question(&account.name)))? {
        Ok(())
    } else {
        Err(refused(&account.name))
    }
}

/// Reads the key of `account` and puts it into `env`, the environment of
/// the claude exec. `parent` is the environment of ccx.
pub fn add_key(env: &mut Env, account: &Account, parent: &Env) -> Result<()> {
    add_key_from(env, account, parent, &keystore::OsStore)
}

pub fn add_key_from(
    env: &mut Env,
    account: &Account,
    parent: &Env,
    store: &dyn SecretStore,
) -> Result<()> {
    let key = keystore::read_key(account, parent, store)
        .map_err(|e| format!("{e}; ccx did not start claude"))?;
    env.set(KEY_VAR, key);
    Ok(())
}

/// The banner of a paid launch. It never shows the key.
pub fn banner(name: &str) -> String {
    format!("ccx → {name} · {KIND_LABEL}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::KeySource;
    use crate::keystore::tests::FakeStore;
    use std::path::PathBuf;

    #[test]
    fn allow_paid_says_yes_with_no_prompt() {
        let asked = std::cell::Cell::new(false);
        let ask = || {
            asked.set(true);
            Ok(false)
        };
        assert!(consent(true, false, ask).unwrap());
        assert!(consent(true, true, || Ok(false)).unwrap());
        assert!(!asked.get());
    }

    #[test]
    fn no_terminal_says_no_with_no_prompt() {
        assert!(!consent(false, false, || panic!("no prompt with no terminal")).unwrap());
    }

    #[test]
    fn a_terminal_asks_the_user() {
        assert!(consent(false, true, || Ok(true)).unwrap());
        assert!(!consent(false, true, || Ok(false)).unwrap());
        assert!(consent(false, true, || Err("eof".into())).is_err());
    }

    #[test]
    fn question_texts() {
        assert_eq!(
            fallback_question("paid"),
            "All subscription accounts are limited. Use paid account paid?"
        );
        assert!(refused("paid").contains("--allow-paid"));
        assert_eq!(banner("paid"), "ccx → paid · api-key (paid)");
    }

    #[test]
    fn add_key_sets_only_the_key_var() {
        let account = Account {
            name: "paid".into(),
            dir: PathBuf::from("/home/test/.claude-paid"),
            dir_text: "~/.claude-paid".into(),
            weight: None,
            aliases: Vec::new(),
            enabled: true,
            email: None,
            own: Vec::new(),
            is_base: false,
            api_key: Some(KeySource::Keyring),
        };
        let store = FakeStore::default();
        store
            .items
            .borrow_mut()
            .insert("ccx-paid".into(), "sk-test".into());
        let parent: Env = [("HOME", "/home/test")].into_iter().collect();
        let mut env = parent.clone();
        add_key_from(&mut env, &account, &parent, &store).unwrap();
        assert_eq!(env.get(KEY_VAR), Some("sk-test".as_ref()));
        assert!(!parent.contains(KEY_VAR));

        let mut env = parent.clone();
        let err = add_key_from(&mut env, &account, &parent, &FakeStore::default()).unwrap_err();
        assert!(err.contains("ccx did not start claude"), "{err}");
        assert!(!env.contains(KEY_VAR));
    }
}

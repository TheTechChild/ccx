//! `ccx manage add NAME --api-key`: make an API-key account, a paid
//! fallback (#20).
//!
//! Add reads the key with a hidden prompt (or one line of stdin when stdin
//! is not a terminal) and stores it in the OS secret store as `ccx-<name>`.
//! With `--key-command`, ccx keeps no copy: it runs the command at each
//! paid launch. When the secret store is not available, ccx offers
//! `key_command` or the key file `~/.config/ccx/keys/<name>` at mode 0600.
//! ccx writes the key file only after a confirm. `config.toml` never gets
//! the key.

use std::io::{BufRead, IsTerminal};
use std::path::Path;

use toml_edit::{DocumentMut, Item, value};

use crate::add::{self, Args};
use crate::adopt;
use crate::config::{self, KeySource, Result};
use crate::env::Env;
use crate::keystore::{self, SecretStore};
use crate::sync;

/// The mode of a new `config.toml`.
const CONFIG_MODE: u32 = 0o644;

/// Runs `ccx manage add NAME --api-key`.
pub fn run(parent: &Env, args: &Args) -> Result<()> {
    let home = config::home(parent)?;
    let config = config::load(parent)?;
    let config_path = config::config_path(parent)?;
    let overlay_path = config::overlay_path(parent)?;
    let cwd = std::env::current_dir().map_err(|e| format!("cannot read the cwd: {e}"))?;
    let name = &args.name;
    let (dir, dir_text) = add::account_dir(name, args.dir.as_deref(), home, &cwd);
    add::check_new(&config, name, &dir)?;
    add::check_dir(&dir)?;
    let key_file = keystore::key_file(parent, name)?;

    // The source that the user asks for. A failing secret store can change
    // `Keyring` to `File` later.
    let wanted = match (&args.key_command, args.key_file) {
        (Some(command), _) => KeySource::Command(command.clone()),
        (None, true) => KeySource::File,
        (None, false) => KeySource::Keyring,
    };
    let mut main = adopt::read_doc(&config_path)?;
    let overlay = adopt::read_doc(&overlay_path)?;
    adopt::add_account(&mut main, name, &dir_text)?;
    set_key_entry(&mut main, name, &wanted)?;
    add::validate(&main, &overlay, &config_path, &overlay_path, home)?;

    println!("ccx manage add will:");
    for line in plan(name, &dir, &wanted, &key_file) {
        println!("  - {line}");
    }
    let retry = format!("ccx manage add {name} --api-key -y");
    if !adopt::confirm(args.yes, "Continue?", &retry)? {
        println!("Nothing changed.");
        return Ok(());
    }

    let source = match &wanted {
        KeySource::Command(command) => {
            // Check the command one time now, so that a bad command does
            // not stop a launch later.
            keystore::run_key_command(name, command, parent)?;
            println!("The key_command gives a key. ccx keeps no copy of the key.");
            wanted.clone()
        }
        _ => {
            let key = read_key_input(name)?;
            let source = store_key(
                name,
                &key,
                wanted == KeySource::File,
                &keystore::OsStore,
                &key_file,
                &mut |path| confirm_file(path),
            )?;
            match source {
                KeySource::File => {
                    println!("Wrote the key to {} (mode 0600)", key_file.display())
                }
                _ => println!(
                    "Stored the key in the OS secret store as `{}`",
                    keystore::service(name)
                ),
            }
            source
        }
    };

    add::make_dir(&dir)?;
    println!("Made {} (mode 0700)", dir.display());
    let sync_plan = sync::plan(&config.base().dir, &dir, &[])?;
    print!("{}", sync::render(name, &sync_plan));
    let applied = sync::apply(&sync_plan, sync::Mode::Full { conflicts: false })?;
    println!("Sync made {} changes.", applied.changed);

    set_key_entry(&mut main, name, &source)?;
    let (main_text, _) = add::validate(&main, &overlay, &config_path, &overlay_path, home)?;
    let mode = std::fs::metadata(&config_path)
        .map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o7777)
        .unwrap_or(CONFIG_MODE);
    let written = adopt::write_atomic(&config_path, &main_text, mode)?;
    println!("Wrote {}", written.display());
    println!(
        "{name}: kind {}. ccx uses it only when all subscription accounts are limited, and asks first.",
        crate::paid::KIND_LABEL
    );
    Ok(())
}

/// The plan lines that add shows before it asks.
pub fn plan(name: &str, dir: &Path, source: &KeySource, key_file: &Path) -> Vec<String> {
    let mut lines = vec![
        format!("make {} with mode 0700", dir.display()),
        "link the shared setup of the base account into it (sync)".to_string(),
    ];
    match source {
        KeySource::Command(_) => {
            lines.push("run the key_command one time to check that it gives a key".to_string())
        }
        KeySource::File => {
            lines.push("read the API key (hidden prompt, or one line of stdin)".to_string());
            lines.push(format!(
                "write the key to {} with mode 0600",
                key_file.display()
            ));
        }
        KeySource::Keyring => {
            lines.push("read the API key (hidden prompt, or one line of stdin)".to_string());
            lines.push(format!(
                "store the key in the OS secret store as `{}`",
                keystore::service(name)
            ));
        }
    }
    lines.push(format!(
        "add account {name} to config.toml with kind = \"api_key\" and no key in it"
    ));
    lines
}

/// Writes `kind = "api_key"` and the key source into `accounts.NAME`.
pub fn set_key_entry(doc: &mut DocumentMut, name: &str, source: &KeySource) -> Result<()> {
    let table = doc
        .get_mut("accounts")
        .and_then(|a| a.get_mut(name))
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| format!("config.toml has no table accounts.{name}"))?;
    table.insert("kind", value("api_key"));
    table.remove("key_store");
    table.remove("key_command");
    match source {
        KeySource::Keyring => table.insert("key_store", value("keyring")),
        KeySource::File => table.insert("key_store", value("file")),
        KeySource::Command(command) => table.insert("key_command", value(command.as_str())),
    };
    Ok(())
}

/// Reads the key: a hidden prompt on a terminal, else one line of stdin.
fn read_key_input(name: &str) -> Result<String> {
    let text = if std::io::stdin().is_terminal() && std::io::stderr().is_terminal() {
        dialoguer::Password::new()
            .with_prompt(format!("API key for account {name}"))
            .interact()
            .map_err(|e| format!("cannot read the key: {e}"))?
    } else {
        let mut line = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(|e| format!("cannot read the key from stdin: {e}"))?;
        line
    };
    keystore::clean_key(&text)
}

/// Asks before ccx writes the key file. With no terminal, the answer is no.
fn confirm_file(path: &Path) -> Result<bool> {
    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        return Ok(false);
    }
    dialoguer::Confirm::new()
        .with_prompt(format!("Write the key to {} (mode 0600)?", path.display()))
        .default(false)
        .interact()
        .map_err(|e| format!("cannot read the answer: {e}"))
}

/// The text when the OS secret store is not available.
pub fn no_store_help(name: &str, error: &str, key_file: &Path) -> String {
    format!(
        "\
ccx: the OS secret store is not available: {error}
ccx: this is usual on a server with no Secret Service (no D-Bus session or no unlocked keyring).
ccx: you can use one of these:
ccx:   1. key_command: ccx runs a command (for example a password manager) and reads the key from its stdout.
ccx:      Run: ccx manage add {name} --api-key --key-command '<command>'
ccx:   2. A key file: ccx writes the key to {} with mode 0600.
ccx:      Run: ccx manage add {name} --api-key --key-file",
        key_file.display()
    )
}

/// Stores `key`. With `use_file`, the key goes to `key_file` (the user
/// asked for it). Else it goes to `store`. When `store` fails, ccx shows
/// the fallbacks and writes the key file only when `confirm_file` says
/// yes. Returns the source to write into the config.
pub fn store_key(
    name: &str,
    key: &str,
    use_file: bool,
    store: &dyn SecretStore,
    key_file: &Path,
    confirm_file: &mut dyn FnMut(&Path) -> Result<bool>,
) -> Result<KeySource> {
    if !use_file {
        match store.set(&keystore::service(name), key) {
            Ok(()) => return Ok(KeySource::Keyring),
            Err(e) => {
                eprintln!("{}", no_store_help(name, &e, key_file));
                if !confirm_file(key_file)? {
                    return Err("ccx did not store the key, and the config did not change. Use key_command or the key file, as shown above".into());
                }
            }
        }
    }
    keystore::write_key_file(key_file, key)?;
    Ok(KeySource::File)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystore::tests::FakeStore;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_working_store_keeps_the_key_and_writes_no_file() {
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("keys/paid");
        let store = FakeStore::default();
        let source = store_key("paid", "sk-test", false, &store, &file, &mut |_| {
            panic!("no file prompt when the store works")
        })
        .unwrap();
        assert_eq!(source, KeySource::Keyring);
        assert_eq!(store.items.borrow()["ccx-paid"], "sk-test");
        assert!(!file.exists());
    }

    #[test]
    fn a_failing_store_offers_the_file_and_writes_it_only_after_a_yes() {
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("keys/paid");
        let store = FakeStore {
            fail: true,
            ..FakeStore::default()
        };
        let mut asked = 0;
        let err = store_key("paid", "sk-test", false, &store, &file, &mut |_| {
            asked += 1;
            Ok(false)
        })
        .unwrap_err();
        assert_eq!(asked, 1);
        assert!(err.contains("did not store the key"), "{err}");
        assert!(!file.exists());

        let source = store_key("paid", "sk-test", false, &store, &file, &mut |_| Ok(true)).unwrap();
        assert_eq!(source, KeySource::File);
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode(&file), 0o600);
        assert_eq!(mode(file.parent().unwrap()), 0o700);
        assert_eq!(keystore::read_key_file(&file).unwrap(), "sk-test");
    }

    #[test]
    fn key_file_flag_skips_the_store() {
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("keys/paid");
        let store = FakeStore::default();
        let source = store_key("paid", "sk-test", true, &store, &file, &mut |_| {
            panic!("the flag is the confirm")
        })
        .unwrap();
        assert_eq!(source, KeySource::File);
        assert!(store.items.borrow().is_empty());
    }

    #[test]
    fn help_names_both_fallbacks() {
        let text = no_store_help("paid", "no D-Bus", Path::new("/h/.config/ccx/keys/paid"));
        assert!(text.contains("--key-command"), "{text}");
        assert!(text.contains("--key-file"), "{text}");
        assert!(text.contains("/h/.config/ccx/keys/paid"), "{text}");
    }

    #[test]
    fn key_entry_has_kind_and_source_and_no_key() {
        let mut doc: DocumentMut = "[accounts.main]\ndir = \"~/.claude\"\n".parse().unwrap();
        adopt::add_account(&mut doc, "paid", "~/.claude-paid").unwrap();
        set_key_entry(&mut doc, "paid", &KeySource::Keyring).unwrap();
        let text = doc.to_string();
        assert!(text.contains("kind = \"api_key\""), "{text}");
        assert!(text.contains("key_store = \"keyring\""), "{text}");
        set_key_entry(&mut doc, "paid", &KeySource::Command("op read x".into())).unwrap();
        let text = doc.to_string();
        assert!(!text.contains("key_store"), "{text}");
        assert!(text.contains("key_command = \"op read x\""), "{text}");
    }

    #[test]
    fn plan_names_the_store() {
        let file = Path::new("/h/keys/paid");
        let dir = Path::new("/h/.claude-paid");
        let text = plan("paid", dir, &KeySource::Keyring, file).join("\n");
        assert!(text.contains("`ccx-paid`"), "{text}");
        let text = plan("paid", dir, &KeySource::File, file).join("\n");
        assert!(text.contains("/h/keys/paid with mode 0600"), "{text}");
        let text = plan("paid", dir, &KeySource::Command("x".into()), file).join("\n");
        assert!(text.contains("key_command"), "{text}");
    }
}

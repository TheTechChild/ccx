//! The keys of API-key accounts (#20).
//!
//! These are keys that ccx stores itself. ccx never reads the login data of
//! claude (ADR 0002). The only secret-store items that ccx touches have
//! a service name that starts with [`SERVICE_PREFIX`].
//!
//! A key comes from one of three places (ADR 0010):
//!
//! - The OS secret store, through the `security` command on macOS and the
//!   `secret-tool` command (libsecret) on Linux. The key goes to these
//!   commands only on stdin, never in an argument.
//! - The file `~/.config/ccx/keys/<name>` at mode 0600, in a dir at mode
//!   0700. ccx refuses to read a key file with a different mode.
//! - The stdout of `key_command`.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::{self, Account, KeySource, Result};
use crate::env::Env;

/// The start of the service name of each secret-store item that ccx
/// makes. ccx never touches a different item.
pub const SERVICE_PREFIX: &str = "ccx-";
/// The account attribute of each secret-store item that ccx makes.
const ITEM_ACCOUNT: &str = "ccx";
/// The mode of a key file.
pub const KEY_FILE_MODE: u32 = 0o600;
/// The mode of the dir of the key files.
pub const KEY_DIR_MODE: u32 = 0o700;

/// The secret-store service name of an account: `ccx-<name>`.
pub fn service(name: &str) -> String {
    format!("{SERVICE_PREFIX}{name}")
}

/// An OS secret store. Tests use a fake store, so no test touches the real
/// store of the user.
pub trait SecretStore {
    /// Stores `key` as the item `service`. It replaces an old item.
    fn set(&self, service: &str, key: &str) -> Result<()>;
    /// Reads the item `service`.
    fn get(&self, service: &str) -> Result<String>;
    /// Deletes the item `service`. It is not an error when the item is
    /// not there.
    fn delete(&self, service: &str) -> Result<()>;
}

/// The secret store of the OS: the macOS keychain through `security`, or
/// the Secret Service through `secret-tool` on Linux.
pub struct OsStore;

impl SecretStore for OsStore {
    fn set(&self, service: &str, key: &str) -> Result<()> {
        check_service(service)?;
        if cfg!(target_os = "macos") {
            // `security -i` reads the command from stdin, so the key is
            // not in the argument list of a process. `-X` gives the key as
            // hex, so the key needs no quotes.
            let line = format!(
                "add-generic-password -U -a {ITEM_ACCOUNT} -s {service} -X {}\n",
                hex(key)
            );
            run("security", &["-i"], Some(&line))?;
            // `security -i` can exit 0 when its command failed.
            match self.get(service) {
                Ok(stored) if stored == key => Ok(()),
                Ok(_) => Err("the keychain item has a different value after the write".into()),
                Err(e) => Err(e),
            }
        } else {
            let label = format!("--label={service}");
            run(
                "secret-tool",
                &["store", &label, "service", service, "account", ITEM_ACCOUNT],
                Some(key),
            )
            .map(drop)
        }
    }

    fn get(&self, service: &str) -> Result<String> {
        check_service(service)?;
        let out = if cfg!(target_os = "macos") {
            run(
                "security",
                &[
                    "find-generic-password",
                    "-a",
                    ITEM_ACCOUNT,
                    "-s",
                    service,
                    "-w",
                ],
                None,
            )?
        } else {
            run(
                "secret-tool",
                &["lookup", "service", service, "account", ITEM_ACCOUNT],
                None,
            )?
        };
        Ok(out.trim_end_matches(['\n', '\r']).to_string())
    }

    fn delete(&self, service: &str) -> Result<()> {
        check_service(service)?;
        if cfg!(target_os = "macos") {
            let args = ["delete-generic-password", "-a", ITEM_ACCOUNT, "-s", service];
            match Command::new("security")
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
            {
                // 44 is errSecItemNotFound: there is nothing to delete.
                Ok(status) if status.success() || status.code() == Some(44) => Ok(()),
                Ok(status) => Err(format!(
                    "`security delete-generic-password` failed ({status})"
                )),
                Err(e) => Err(format!("cannot run security: {e}")),
            }
        } else {
            run(
                "secret-tool",
                &["clear", "service", service, "account", ITEM_ACCOUNT],
                None,
            )
            .map(drop)
        }
    }
}

/// Refuses a service name that ccx did not make.
fn check_service(service: &str) -> Result<()> {
    match service.strip_prefix(SERVICE_PREFIX) {
        Some(name) if config::valid_name(name) => Ok(()),
        _ => Err(format!("`{service}` is not a ccx secret-store item")),
    }
}

/// Runs a secret-store command. `input` goes to stdin. Returns stdout.
/// stdout is never part of an error, because it can be a key.
fn run(program: &str, args: &[&str], input: Option<&str>) -> Result<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| format!("cannot write to {program}: {e}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    if !out.status.success() {
        let error = String::from_utf8_lossy(&out.stderr);
        let error = error.trim();
        return Err(if error.is_empty() {
            format!("{program} failed ({})", out.status)
        } else {
            format!("{program} failed ({}): {error}", out.status)
        });
    }
    String::from_utf8(out.stdout).map_err(|_| format!("{program} gave a value that is not UTF-8"))
}

fn hex(text: &str) -> String {
    text.bytes().map(|b| format!("{b:02x}")).collect()
}

/// Checks a key and removes the whitespace at its start and end. A key
/// cannot be empty or have whitespace or control characters in it.
pub fn clean_key(key: &str) -> Result<String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("the API key is empty".into());
    }
    if key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("the API key has spaces or control characters in it".into());
    }
    Ok(key.to_string())
}

/// The dir of the key files: `~/.config/ccx/keys`.
pub fn key_dir(env: &Env) -> Result<PathBuf> {
    Ok(config::config_dir(env)?.join("keys"))
}

/// The key file of an account: `~/.config/ccx/keys/<name>`.
pub fn key_file(env: &Env, name: &str) -> Result<PathBuf> {
    Ok(key_dir(env)?.join(name))
}

/// Writes `key` to `path` at mode 0600. The dir gets mode 0700. The write
/// goes to a temp file first, so a reader never sees a part of a key.
pub fn write_key_file(path: &Path, key: &str) -> Result<()> {
    let dir = path.parent().ok_or("the key file has no parent dir")?;
    let error = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
    fs::DirBuilder::new()
        .recursive(true)
        .mode(KEY_DIR_MODE)
        .create(dir)
        .map_err(error)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(KEY_DIR_MODE)).map_err(error)?;
    let name = path.file_name().ok_or("the key file has no name")?;
    let temp = dir.join(format!(".{}.tmp", name.to_string_lossy()));
    let _ = fs::remove_file(&temp);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(KEY_FILE_MODE)
        .open(&temp)
        .map_err(error)?;
    let written = file
        .write_all(key.as_bytes())
        .and_then(|()| file.set_permissions(fs::Permissions::from_mode(KEY_FILE_MODE)))
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::rename(&temp, path));
    if let Err(e) = written {
        let _ = fs::remove_file(&temp);
        return Err(error(e));
    }
    Ok(())
}

/// Reads a key file. It refuses a file with a mode that is not 0600.
pub fn read_key_file(path: &Path) -> Result<String> {
    let meta = fs::metadata(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mode = meta.permissions().mode() & 0o7777;
    if mode != KEY_FILE_MODE {
        return Err(format!(
            "{} has mode {mode:04o}; ccx reads a key file only at mode 0600. To fix it, run: chmod 600 '{}'",
            path.display(),
            path.display()
        ));
    }
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    clean_key(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Runs `key_command` with `sh -c` and returns the key from its stdout.
/// stdin and stderr stay on the terminal, so a password manager can ask
/// for a password.
pub fn run_key_command(name: &str, command: &str, env: &Env) -> Result<String> {
    let out = Command::new("sh")
        .args(["-c", command])
        .env_clear()
        .envs(env.iter())
        .stdout(Stdio::piped())
        .output()
        .map_err(|e| format!("cannot run the key_command of account {name}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "the key_command of account {name} failed ({})",
            out.status
        ));
    }
    let text = String::from_utf8(out.stdout)
        .map_err(|_| format!("the key_command of account {name} gave a key that is not UTF-8"))?;
    if text.trim().is_empty() {
        return Err(format!(
            "the key_command of account {name} gave no key on stdout"
        ));
    }
    clean_key(&text).map_err(|e| format!("the key_command of account {name}: {e}"))
}

/// Reads the key of an API-key account. `env` is the environment of ccx.
pub fn read_key(account: &Account, env: &Env, store: &dyn SecretStore) -> Result<String> {
    let name = &account.name;
    let source = account
        .api_key
        .as_ref()
        .ok_or_else(|| format!("account {name} is not an API-key account"))?;
    match source {
        KeySource::Keyring => store
            .get(&service(name))
            .and_then(|key| clean_key(&key))
            .map_err(|e| {
                format!(
                    "cannot read the API key of account {name} from the OS secret store (item `{}`): {e}",
                    service(name)
                )
            }),
        KeySource::File => read_key_file(&key_file(env, name)?),
        KeySource::Command(command) => run_key_command(name, command, env),
    }
}

/// What [`delete_key`] removes, as text for a plan.
pub fn delete_plan(account: &Account, env: &Env) -> Result<Option<String>> {
    Ok(match &account.api_key {
        None | Some(KeySource::Command(_)) => None,
        Some(KeySource::Keyring) => Some(format!(
            "delete the API key: the OS secret store item `{}`",
            service(&account.name)
        )),
        Some(KeySource::File) => Some(format!(
            "delete the API key file {}",
            key_file(env, &account.name)?.display()
        )),
    })
}

/// Deletes the key that ccx stores for an API-key account: the secret
/// store item `ccx-<name>` or the key file. Call it only after the user
/// confirms. A `key_command` account has no key that ccx stores. Returns a
/// line for the user.
pub fn delete_key(account: &Account, env: &Env, store: &dyn SecretStore) -> Result<String> {
    let name = &account.name;
    match &account.api_key {
        None => Err(format!("account {name} is not an API-key account")),
        Some(KeySource::Command(_)) => Ok(format!(
            "The API key of account {name} comes from key_command. ccx keeps no copy of it."
        )),
        Some(KeySource::Keyring) => {
            store.delete(&service(name))?;
            Ok(format!(
                "Deleted the OS secret store item `{}`.",
                service(name)
            ))
        }
        Some(KeySource::File) => {
            let path = key_file(env, name)?;
            match fs::remove_file(&path) {
                Ok(()) => Ok(format!("Deleted {}", path.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    Ok(format!("{} does not exist", path.display()))
                }
                Err(e) => Err(format!("cannot delete {}: {e}", path.display())),
            }
        }
    }
}

/// The command that deletes the key of an account by hand, for an account
/// that `rm` removes without `--purge`.
pub fn manual_delete(account: &Account, env: &Env) -> Result<Option<String>> {
    let service = service(&account.name);
    Ok(match &account.api_key {
        None | Some(KeySource::Command(_)) => None,
        Some(KeySource::Keyring) if cfg!(target_os = "macos") => Some(format!(
            "security delete-generic-password -a {ITEM_ACCOUNT} -s '{service}'"
        )),
        Some(KeySource::Keyring) => Some(format!(
            "secret-tool clear service '{service}' account {ITEM_ACCOUNT}"
        )),
        Some(KeySource::File) => Some(format!("rm '{}'", key_file(env, &account.name)?.display())),
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// A secret store in memory. With `fail`, each call fails like a
    /// Linux system with no Secret Service.
    #[derive(Default)]
    pub struct FakeStore {
        pub items: RefCell<BTreeMap<String, String>>,
        pub fail: bool,
    }

    impl SecretStore for FakeStore {
        fn set(&self, service: &str, key: &str) -> Result<()> {
            check_service(service)?;
            if self.fail {
                return Err("Cannot autolaunch D-Bus without X11 $DISPLAY".into());
            }
            self.items
                .borrow_mut()
                .insert(service.to_string(), key.to_string());
            Ok(())
        }
        fn get(&self, service: &str) -> Result<String> {
            if self.fail {
                return Err("no secret service".into());
            }
            self.items
                .borrow()
                .get(service)
                .cloned()
                .ok_or_else(|| "no such item".to_string())
        }
        fn delete(&self, service: &str) -> Result<()> {
            if self.fail {
                return Err("no secret service".into());
            }
            self.items.borrow_mut().remove(service);
            Ok(())
        }
    }

    fn account(name: &str, source: Option<KeySource>) -> Account {
        Account {
            name: name.into(),
            dir: PathBuf::from(format!("/home/test/.claude-{name}")),
            dir_text: format!("~/.claude-{name}"),
            weight: None,
            aliases: Vec::new(),
            enabled: true,
            email: None,
            own: Vec::new(),
            is_base: false,
            api_key: source,
        }
    }

    fn env_for(home: &Path) -> Env {
        [
            ("HOME", home.as_os_str()),
            ("PATH", "/usr/bin:/bin".as_ref()),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn service_names_start_with_ccx() {
        assert_eq!(service("paid"), "ccx-paid");
        assert!(check_service("ccx-paid").is_ok());
        for bad in ["Claude Code-credentials", "ccx-", "ccx-Bad", "paid"] {
            assert!(check_service(bad).is_err(), "{bad}");
        }
        // The OS store refuses an item that ccx did not make, before it
        // runs a command.
        assert!(OsStore.get("Claude Code-credentials").is_err());
    }

    #[test]
    fn clean_key_trims_and_refuses_bad_keys() {
        assert_eq!(clean_key("  sk-test-1\n").unwrap(), "sk-test-1");
        assert!(clean_key("\n").is_err());
        assert!(clean_key("sk test").is_err());
        assert!(clean_key("sk\u{7}x").is_err());
    }

    #[test]
    fn hex_encodes_each_byte() {
        assert_eq!(hex("sk-A"), "736b2d41");
    }

    #[test]
    fn key_file_is_0600_in_a_0700_dir_and_other_modes_are_refused() {
        let home = tempfile::tempdir().unwrap();
        let env = env_for(home.path());
        let path = key_file(&env, "paid").unwrap();
        assert_eq!(path, home.path().join(".config/ccx/keys/paid"));
        write_key_file(&path, "sk-test-file").unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        assert_eq!(read_key_file(&path).unwrap(), "sk-test-file");
        // A second write replaces the key.
        write_key_file(&path, "sk-test-new").unwrap();
        assert_eq!(read_key_file(&path).unwrap(), "sk-test-new");

        for bad in [0o644, 0o640, 0o400] {
            fs::set_permissions(&path, fs::Permissions::from_mode(bad)).unwrap();
            let err = read_key_file(&path).unwrap_err();
            assert!(err.contains("only at mode 0600"), "{err}");
            assert!(!err.contains("sk-test"), "{err}");
        }
    }

    #[test]
    fn key_command_gives_the_key_or_a_clear_error() {
        let home = tempfile::tempdir().unwrap();
        let env = env_for(home.path());
        assert_eq!(
            run_key_command("paid", "printf 'sk-test-cmd\\n'", &env).unwrap(),
            "sk-test-cmd"
        );
        let err = run_key_command("paid", "exit 3", &env).unwrap_err();
        assert!(err.contains("key_command of account paid failed"), "{err}");
        let err = run_key_command("paid", "true", &env).unwrap_err();
        assert!(err.contains("gave no key"), "{err}");
        let err = run_key_command("paid", "printf 'two words'", &env).unwrap_err();
        assert!(!err.contains("two words"), "{err}");
    }

    #[test]
    fn read_and_delete_for_each_source() {
        let home = tempfile::tempdir().unwrap();
        let env = env_for(home.path());
        let store = FakeStore::default();
        store.set("ccx-paid", "sk-test-store").unwrap();

        let keyring = account("paid", Some(KeySource::Keyring));
        assert_eq!(read_key(&keyring, &env, &store).unwrap(), "sk-test-store");
        assert!(
            delete_key(&keyring, &env, &store)
                .unwrap()
                .contains("ccx-paid")
        );
        assert!(store.items.borrow().is_empty());
        let err = read_key(&keyring, &env, &store).unwrap_err();
        assert!(err.contains("item `ccx-paid`"), "{err}");

        let file = account("paidfile", Some(KeySource::File));
        let path = key_file(&env, "paidfile").unwrap();
        write_key_file(&path, "sk-test-file").unwrap();
        assert_eq!(read_key(&file, &env, &store).unwrap(), "sk-test-file");
        delete_key(&file, &env, &store).unwrap();
        assert!(!path.exists());
        assert!(
            delete_key(&file, &env, &store)
                .unwrap()
                .contains("does not exist")
        );

        let command = account("paidcmd", Some(KeySource::Command("printf sk-c".into())));
        assert_eq!(read_key(&command, &env, &store).unwrap(), "sk-c");
        assert!(
            delete_key(&command, &env, &store)
                .unwrap()
                .contains("no copy")
        );
        assert_eq!(delete_plan(&command, &env).unwrap(), None);

        let subscription = account("sub", None);
        assert!(read_key(&subscription, &env, &store).is_err());
        assert!(delete_key(&subscription, &env, &store).is_err());
    }

    #[test]
    fn a_failing_store_gives_an_error_and_no_panic() {
        let store = FakeStore {
            fail: true,
            ..FakeStore::default()
        };
        assert!(store.set("ccx-paid", "sk").is_err());
        let home = tempfile::tempdir().unwrap();
        let env = env_for(home.path());
        let err = read_key(&account("paid", Some(KeySource::Keyring)), &env, &store).unwrap_err();
        assert!(err.contains("OS secret store"), "{err}");
    }
}

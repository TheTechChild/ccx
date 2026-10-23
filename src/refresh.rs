//! Stale-while-revalidate (#11): the refresh lock, the 60-second rule, the
//! blocking fetch of a launch and the detached background refresh.
//!
//! All usage fetches use one file lock, `~/.local/state/ccx/refresh.lock`.
//! The lock is an advisory `flock` lock that the OS releases when the file
//! closes, also when the process stops. ccx records the start time of the
//! last fetch in `~/.local/state/ccx/refresh.json`.
//!
//! - A background refresh exits with no fetch when a different process
//!   holds the lock, or when the last fetch started less than
//!   `usage.refresh_min_gap_secs` ago.
//! - A blocking fetch waits for the lock within its time limit. It does not
//!   use the 60-second rule.
//! - A manual refresh waits for the lock. It does not use the 60-second
//!   rule.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::config::{self, Account, Result};
use crate::env::Env;
use crate::launch;
use crate::usage::{self, Snapshot};

/// How often a blocking wait tries the lock again.
const POLL: Duration = Duration::from_millis(20);

/// `~/.local/state/ccx/refresh.lock`.
pub fn lock_path(home: &OsStr) -> PathBuf {
    usage::state_dir(home).join("refresh.lock")
}

/// `~/.local/state/ccx/refresh.json`: the start time of the last fetch.
pub fn last_start_path(home: &OsStr) -> PathBuf {
    usage::state_dir(home).join("refresh.json")
}

/// The refresh lock. The lock is released when this value is dropped.
#[derive(Debug)]
pub struct Lock {
    _file: File,
}

fn open_lock(home: &OsStr) -> Result<File> {
    let path = lock_path(home);
    let error = |e: std::io::Error| format!("cannot open {}: {e}", path.display());
    usage::make_private_dir(path.parent().expect("the lock has a parent")).map_err(error)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)
        .map_err(error)
}

/// Takes the lock with no wait. `None` when a different process holds it.
pub fn try_lock(home: &OsStr) -> Result<Option<Lock>> {
    let file = open_lock(home)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(Lock { _file: file })),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(format!("cannot lock the refresh lock: {e}")),
    }
}

/// Waits for the lock until `deadline`. `None` when the time ends first.
pub fn lock_until(home: &OsStr, deadline: Instant) -> Result<Option<Lock>> {
    let file = open_lock(home)?;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Some(Lock { _file: file })),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())));
            }
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(e)) => {
                return Err(format!("cannot lock the refresh lock: {e}"));
            }
        }
    }
}

/// Waits for the lock with no time limit.
pub fn lock_wait(home: &OsStr) -> Result<Lock> {
    let file = open_lock(home)?;
    file.lock()
        .map_err(|e| format!("cannot lock the refresh lock: {e}"))?;
    Ok(Lock { _file: file })
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LastStart {
    started_at_ms: Option<u64>,
}

/// The start time of the last fetch, or `None` when it is not known.
pub fn last_start(home: &OsStr) -> Option<u64> {
    let bytes = fs::read(last_start_path(home)).ok()?;
    serde_json::from_slice::<LastStart>(&bytes)
        .ok()?
        .started_at_ms
}

/// Records `now_ms` as the start time of the last fetch.
pub fn record_start(home: &OsStr, now_ms: u64) -> Result<()> {
    let text = serde_json::to_string(&LastStart {
        started_at_ms: Some(now_ms),
    })
    .expect("the start time always serializes");
    usage::write_private(&last_start_path(home), text.as_bytes())
}

/// True when usage data from `fetched_at_ms` is less than `fresh_minutes`
/// old at `now_ms`.
pub fn is_fresh(fetched_at_ms: u64, now_ms: u64, fresh_minutes: u64) -> bool {
    now_ms.saturating_sub(fetched_at_ms) < fresh_minutes.saturating_mul(60_000)
}

/// True when a background refresh can run: no fetch started in the last
/// `min_gap_secs`. A start time in the future counts as now.
pub fn gap_passed(last_start_ms: Option<u64>, now_ms: u64, min_gap_secs: u64) -> bool {
    last_start_ms.is_none_or(|last| now_ms.saturating_sub(last) >= min_gap_secs * 1000)
}

/// True when `CCX_NO_REFRESH=1`: a launch fetches nothing and starts no
/// background refresh.
pub fn disabled(env: &Env) -> bool {
    env.get("CCX_NO_REFRESH") == Some("1".as_ref())
}

/// Runs the `/usage` fetch for each account at the same time and writes
/// each result to the cache. The results are in the order of `accounts`.
pub fn fetch_all(
    accounts: &[&Account],
    claude: &Path,
    env: &Env,
    home: &OsStr,
    depth: u32,
    timeout: Duration,
) -> Vec<Result<Snapshot>> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = accounts
            .iter()
            .map(|account| {
                scope.spawn(move || {
                    let (account_env, _) = launch::build_env(env, account, depth);
                    let snapshot = usage::fetch(claude, &account_env, home, timeout)?;
                    usage::write_cache(&usage::cache_path(home, &account.name), &snapshot)?;
                    Ok(snapshot)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("a fetch thread does not panic"))
            .collect()
    })
}

/// The blocking fetch of a launch for the accounts in `stale`. It waits
/// for the lock and runs the fetches within one time limit of
/// `usage.fetch_timeout_secs`. After it gets the lock, it reads the cache
/// again and skips an account that a different process refreshed. It never
/// fails: on an error or at the time limit, the launch uses the old data.
/// The lock is released before this function returns.
pub fn blocking(
    config: &config::Config,
    stale: &[&Account],
    claude: &Path,
    env: &Env,
    home: &OsStr,
    depth: u32,
) {
    let usage_config = &config.usage;
    let deadline = Instant::now() + Duration::from_secs(usage_config.fetch_timeout_secs);
    let Ok(Some(lock)) = lock_until(home, deadline) else {
        return;
    };
    let now = usage::now_ms();
    let still_stale: Vec<&Account> = stale
        .iter()
        .copied()
        .filter(|a| {
            usage::read_cache(&usage::cache_path(home, &a.name))
                .is_none_or(|s| !is_fresh(s.fetched_at_ms, now, usage_config.fresh_minutes))
        })
        .collect();
    let remaining = deadline.saturating_duration_since(Instant::now());
    if still_stale.is_empty() || remaining.is_zero() {
        return;
    }
    let _ = record_start(home, now);
    fetch_all(&still_stale, claude, env, home, depth, remaining);
    drop(lock);
}

/// Starts `ccx usage refresh --all --background` as a detached process.
/// A short-lived `sh` starts the refresh in the background and exits, so
/// the refresh is not a child of `claude` after the exec. The refresh is in
/// its own process group and has no terminal input or output.
pub fn spawn_background(ccx: &Path) -> std::io::Result<()> {
    let status = Command::new("/bin/sh")
        .arg("-c")
        .arg(r#""$0" usage refresh --all --background </dev/null >/dev/null 2>&1 &"#)
        .arg(ccx)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("sh exited with {status}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: u64 = 60_000;

    #[test]
    fn fresh_minutes_sets_the_cache_age_limit() {
        let now = 100 * MINUTE;
        assert!(is_fresh(now - 14 * MINUTE, now, 15));
        assert!(!is_fresh(now - 15 * MINUTE, now, 15));
        assert!(!is_fresh(now - 16 * MINUTE, now, 15));
        // A different value in `[usage]` changes the limit.
        assert!(is_fresh(now - 16 * MINUTE, now, 30));
        assert!(!is_fresh(now - 2 * MINUTE, now, 1));
        assert!(!is_fresh(now, now, 0));
        // Data from the future counts as fresh.
        assert!(is_fresh(now + MINUTE, now, 15));
    }

    #[test]
    fn refresh_min_gap_secs_sets_the_background_gap() {
        let now = 1_000_000;
        assert!(gap_passed(None, now, 60));
        assert!(!gap_passed(Some(now - 59_999), now, 60));
        assert!(gap_passed(Some(now - 60_000), now, 60));
        // A different value in `[usage]` changes the gap.
        assert!(!gap_passed(Some(now - 60_000), now, 120));
        assert!(gap_passed(Some(now - 10_000), now, 10));
        assert!(gap_passed(Some(now), now, 0));
        assert!(!gap_passed(Some(now + 5_000), now, 60));
    }

    #[test]
    fn fetch_timeout_secs_sets_the_blocking_limit() {
        // The fetch, the lock and the gap all come from the `[usage]` table.
        let home = tempfile::tempdir().unwrap();
        fs::create_dir(home.path().join(".claude")).unwrap();
        let text = "[accounts.main]\ndir = \"~/.claude\"\n\
                    [usage]\nfresh_minutes = 2\nfetch_timeout_secs = 1\nrefresh_min_gap_secs = 3\n";
        let path = home.path().join("config.toml");
        let config = config::parse(
            &config::Source { path: &path, text },
            None,
            home.path().as_os_str(),
        )
        .unwrap();
        let home = home.path().as_os_str();
        // A different process holds the lock: here a second open file.
        let held = try_lock(home).unwrap().expect("the lock is free");
        let start = Instant::now();
        blocking(
            &config,
            &[config.base()],
            Path::new("/nonexistent/claude"),
            &Env::default(),
            home,
            0,
        );
        let waited = start.elapsed();
        assert!(waited >= Duration::from_millis(900), "{waited:?}");
        assert!(waited < Duration::from_secs(3), "{waited:?}");
        assert_eq!(last_start(home), None);
        drop(held);
    }

    #[test]
    fn the_lock_is_exclusive_between_open_files() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path().as_os_str();
        let first = try_lock(home).unwrap().expect("the lock is free");
        assert!(try_lock(home).unwrap().is_none());
        let soon = Instant::now() + Duration::from_millis(100);
        assert!(lock_until(home, soon).unwrap().is_none());
        drop(first);
        // A child process that a different test starts can hold a copy of
        // the lock file for a short time, until it runs its program. Thus
        // wait a little for the lock and do not use `try_lock`.
        let later = Instant::now() + Duration::from_secs(5);
        assert!(lock_until(home, later).unwrap().is_some());
    }

    #[test]
    fn the_start_time_is_written_and_read() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path().as_os_str();
        assert_eq!(last_start(home), None);
        record_start(home, 1234).unwrap();
        assert_eq!(last_start(home), Some(1234));
        fs::write(last_start_path(home), "{bad").unwrap();
        assert_eq!(last_start(home), None);
    }

    #[test]
    fn ccx_no_refresh_turns_off_launch_fetches() {
        let env = |v: &str| -> Env { [("CCX_NO_REFRESH", v)].into_iter().collect() };
        assert!(disabled(&env("1")));
        assert!(!disabled(&env("0")));
        assert!(!disabled(&Env::default()));
    }
}

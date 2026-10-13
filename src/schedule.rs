//! The adaptive interval of the scheduled refresh (#22).
//!
//! The service starts `ccx usage refresh --all --background --scheduled`
//! each `TICK_SECS`. Each run reads the usage cache and decides for each
//! account if a fetch is due. An account is due at `fetched_at_ms` plus its
//! interval. `next_interval` calculates the interval from the cache data:
//!
//! - The highest percent: near `NEAR_PERCENT`, the interval is short. Far
//!   from it, the interval is long.
//! - The change since the fetch before: when the percent increases, the
//!   interval is at most half of the time to `NEAR_PERCENT` at that rate.
//! - The soonest `resets_at` of a window that is not at 0%: the interval
//!   ends at the reset. When a window has reset, the interval is the
//!   minimum.
//! - The last ccx launch on the account: in the last hour, the interval is
//!   half.
//!
//! The result is in `[usage] min_interval_secs..=max_interval_secs`. With
//! `[usage] adaptive = false`, the interval is always `FIXED_INTERVAL_SECS`.
//!
//! Each run writes the next due time of each account to
//! `~/.local/state/ccx/schedule.json`. A launch writes its time to
//! `~/.local/state/ccx/launches/<name>`.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::{self, Result};
use crate::usage::{self, Snapshot};

/// The time between two runs of the service.
pub const TICK_SECS: u32 = 60;
/// The interval when `[usage] adaptive = false`.
pub const FIXED_INTERVAL_SECS: u64 = 300;
/// The percent at which an account is near a limit.
const NEAR_PERCENT: f64 = 90.0;
/// A launch less old than this makes the account active.
const ACTIVE_MS: u64 = 3_600_000;

/// The highest percent of one fetch.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub fetched_at_ms: u64,
    pub max_percent: f64,
}

impl Point {
    pub fn of(snapshot: &Snapshot) -> Point {
        Point {
            fetched_at_ms: snapshot.fetched_at_ms,
            max_percent: max_percent(snapshot, snapshot.fetched_at_ms),
        }
    }
}

/// The highest percent of the windows that have not reset at `now_ms`, or
/// 0 when there is no such window.
pub fn max_percent(snapshot: &Snapshot, now_ms: u64) -> f64 {
    snapshot
        .limits
        .iter()
        .filter(|l| !l.has_reset(now_ms))
        .map(|l| l.percent)
        .fold(0.0, f64::max)
}

/// The interval in seconds after the fetch of `snapshot`. `previous` is the
/// fetch before it. `last_launch_ms` is the last ccx launch on the account.
pub fn next_interval(
    snapshot: &Snapshot,
    previous: Option<Point>,
    last_launch_ms: Option<u64>,
    now_ms: u64,
    usage: &config::Usage,
) -> u64 {
    if !usage.adaptive {
        return FIXED_INTERVAL_SECS;
    }
    let (min, max) = (usage.min_interval_secs, usage.max_interval_secs);
    if snapshot.limits.iter().any(|l| l.has_reset(now_ms)) {
        return min;
    }
    let percent = max_percent(snapshot, now_ms);
    let headroom = ((NEAR_PERCENT - percent) / NEAR_PERCENT).clamp(0.0, 1.0);
    let mut secs = min as f64 + (max - min) as f64 * headroom;

    if let Some(previous) = previous
        && snapshot.fetched_at_ms > previous.fetched_at_ms
        && percent > previous.max_percent
    {
        let elapsed = (snapshot.fetched_at_ms - previous.fetched_at_ms) as f64 / 1000.0;
        let rate = (percent - previous.max_percent) / elapsed;
        secs = secs.min((NEAR_PERCENT - percent).max(0.0) / rate / 2.0);
    }

    if last_launch_ms.is_some_and(|at| now_ms.saturating_sub(at) < ACTIVE_MS) {
        secs /= 2.0;
    }

    let soonest_reset = snapshot
        .limits
        .iter()
        .filter(|l| l.percent > 0.0)
        .filter_map(|l| usage::parse_rfc3339_ms(l.resets_at.as_deref()?))
        .min();
    if let Some(reset) = soonest_reset {
        let to_reset = (reset - snapshot.fetched_at_ms as i64).max(0) as f64 / 1000.0;
        secs = secs.min(to_reset.ceil());
    }

    (secs.round() as u64).clamp(min, max)
}

/// The schedule of one account.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// The newest fetch in the cache.
    pub latest: Option<Point>,
    /// The fetch before `latest`.
    pub previous: Option<Point>,
    /// The start of the last scheduled fetch, also when it failed.
    pub last_attempt_ms: Option<u64>,
    /// The time when the next fetch is due.
    pub next_due_ms: u64,
    /// The interval after `latest`. `None` when the cache is empty.
    pub interval_secs: Option<u64>,
}

/// `~/.local/state/ccx/schedule.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub accounts: BTreeMap<String, Entry>,
}

impl State {
    /// Calculates the next due time of the account `name` from its cache
    /// and records it. Returns the due time.
    ///
    /// When the cache has a new fetch, the old fetch becomes `previous`.
    /// After a failed fetch, the account is due again after the minimum
    /// interval (with `adaptive = false`, after the fixed interval).
    pub fn update(
        &mut self,
        name: &str,
        cache: Option<&Snapshot>,
        last_launch_ms: Option<u64>,
        now_ms: u64,
        usage: &config::Usage,
    ) -> u64 {
        let entry = self.accounts.entry(name.to_string()).or_default();
        let (due, interval) = match cache {
            None => (0, None),
            Some(snapshot) => {
                let point = Point::of(snapshot);
                if entry.latest.map(|p| p.fetched_at_ms) != Some(point.fetched_at_ms) {
                    entry.previous = entry
                        .latest
                        .filter(|p| p.fetched_at_ms < point.fetched_at_ms);
                    entry.latest = Some(point);
                }
                let secs = next_interval(snapshot, entry.previous, last_launch_ms, now_ms, usage);
                (snapshot.fetched_at_ms + secs * 1000, Some(secs))
            }
        };
        let retry_secs = if usage.adaptive {
            usage.min_interval_secs
        } else {
            FIXED_INTERVAL_SECS
        };
        let retry = entry.last_attempt_ms.map_or(0, |at| at + retry_secs * 1000);
        entry.next_due_ms = due.max(retry);
        entry.interval_secs = interval;
        entry.next_due_ms
    }

    /// Keeps only the accounts in `names`.
    pub fn retain(&mut self, names: &[&str]) {
        self.accounts
            .retain(|name, _| names.contains(&name.as_str()));
    }
}

pub fn state_path(home: &OsStr) -> PathBuf {
    usage::state_dir(home).join("schedule.json")
}

/// Reads the state. A missing or bad file gives the default state.
pub fn read_state(home: &OsStr) -> State {
    fs::read(state_path(home))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Writes the state atomically at mode 0600.
pub fn write_state(home: &OsStr, state: &State) -> Result<()> {
    let text = serde_json::to_string_pretty(state).expect("the schedule always serializes");
    usage::write_private(&state_path(home), text.as_bytes())
}

/// `~/.local/state/ccx/launches/<name>`: the time of the last launch.
pub fn launch_path(home: &OsStr, name: &str) -> PathBuf {
    usage::state_dir(home).join("launches").join(name)
}

/// The time of the last ccx launch on the account `name`.
pub fn last_launch(home: &OsStr, name: &str) -> Option<u64> {
    fs::read_to_string(launch_path(home, name))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Records a launch on the account `name` at `now_ms`. This is on the
/// launch path, so it does not sync the file to disk. A reader that sees
/// a partial file gets no time, which is safe.
pub fn record_launch(home: &OsStr, name: &str, now_ms: u64) -> std::io::Result<()> {
    let path = launch_path(home, name);
    usage::make_private_dir(path.parent().expect("a launch path has a parent"))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)?;
    writeln!(file, "{now_ms}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{Kind, Limit, Source};

    const SEC: u64 = 1000;
    const MIN: u64 = 60 * SEC;
    const HOUR: u64 = 60 * MIN;
    /// 2026-01-01T00:00:00Z.
    const NOW: u64 = 1_767_225_600_000;

    fn cfg() -> config::Usage {
        config::Usage {
            fresh_minutes: 15,
            fetch_timeout_secs: 5,
            refresh_min_gap_secs: 60,
            adaptive: true,
            min_interval_secs: 120,
            max_interval_secs: 1800,
        }
    }

    fn rfc3339(ms: u64) -> String {
        // Only for whole seconds after NOW and within one day.
        let secs = (ms - NOW) / SEC;
        assert!(secs < 86_400);
        format!(
            "2026-01-01T{:02}:{:02}:{:02}Z",
            secs / 3600,
            secs % 3600 / 60,
            secs % 60
        )
    }

    /// A snapshot fetched at `fetched_at` with one session window.
    fn snap(fetched_at: u64, percent: f64, resets_at: Option<u64>) -> Snapshot {
        Snapshot {
            source: Source::Usage,
            fetched_at_ms: fetched_at,
            limits: vec![
                Limit {
                    kind: Kind::Session,
                    percent,
                    resets_at: resets_at.map(rfc3339),
                    scope: None,
                },
                Limit {
                    kind: Kind::WeeklyAll,
                    percent: 0.0,
                    resets_at: Some("2026-01-05T00:00:00Z".into()),
                    scope: None,
                },
            ],
        }
    }

    fn interval(s: &Snapshot, previous: Option<Point>, launch: Option<u64>, now: u64) -> u64 {
        next_interval(s, previous, launch, now, &cfg())
    }

    #[test]
    fn an_idle_account_far_from_its_limits_gets_the_maximum() {
        let s = snap(NOW, 0.0, None);
        assert_eq!(interval(&s, None, None, NOW), 1800);
        let previous = Point {
            fetched_at_ms: NOW - 30 * MIN,
            max_percent: 0.0,
        };
        assert_eq!(interval(&s, Some(previous), None, NOW), 1800);
        // A launch more than one hour ago does not count.
        assert_eq!(interval(&s, None, Some(NOW - 2 * HOUR), NOW), 1800);
    }

    #[test]
    fn a_near_limit_account_gets_the_minimum() {
        assert_eq!(
            interval(&snap(NOW, 90.0, Some(NOW + 3 * HOUR)), None, None, NOW),
            120
        );
        assert_eq!(
            interval(&snap(NOW, 99.0, Some(NOW + 3 * HOUR)), None, None, NOW),
            120
        );
        // Half way to the limit: half way between min and max.
        assert_eq!(
            interval(&snap(NOW, 45.0, Some(NOW + 3 * HOUR)), None, None, NOW),
            960
        );
        // The interval gets shorter as the percent increases.
        let mut last = u64::MAX;
        for percent in [0.0, 20.0, 40.0, 60.0, 80.0, 90.0] {
            let secs = interval(&snap(NOW, percent, Some(NOW + 3 * HOUR)), None, None, NOW);
            assert!(secs <= last, "{percent}: {secs} > {last}");
            last = secs;
        }
    }

    #[test]
    fn a_fast_rise_shortens_the_interval() {
        let s = snap(NOW, 50.0, Some(NOW + 3 * HOUR));
        let calm = interval(&s, None, None, NOW);
        assert_eq!(calm, 867);
        // 10 points in 10 minutes: 40 points to go take 40 minutes. Half
        // of that is 20 minutes, which is more than 867 s.
        let slow = Point {
            fetched_at_ms: NOW - 10 * MIN,
            max_percent: 40.0,
        };
        assert_eq!(interval(&s, Some(slow), None, NOW), calm);
        // 20 points in 5 minutes: 40 points to go take 10 minutes. Half of
        // that is 300 s.
        let fast = Point {
            fetched_at_ms: NOW - 5 * MIN,
            max_percent: 30.0,
        };
        assert_eq!(interval(&s, Some(fast), None, NOW), 300);
        // A very fast rise gives the minimum.
        let very_fast = Point {
            fetched_at_ms: NOW - MIN,
            max_percent: 10.0,
        };
        assert_eq!(interval(&s, Some(very_fast), None, NOW), 120);
        // A fall does not shorten the interval.
        let fall = Point {
            fetched_at_ms: NOW - MIN,
            max_percent: 80.0,
        };
        assert_eq!(interval(&s, Some(fall), None, NOW), calm);
    }

    #[test]
    fn a_reset_soon_ends_the_interval_at_the_reset() {
        let s = snap(NOW, 10.0, Some(NOW + 10 * MIN));
        assert_eq!(interval(&s, None, None, NOW), 600);
        // A reset before the minimum gives the minimum.
        let s = snap(NOW, 10.0, Some(NOW + 30 * SEC));
        assert_eq!(interval(&s, None, None, NOW), 120);
        // A window at 0% gains nothing from a reset.
        let s = snap(NOW, 0.0, Some(NOW + 10 * MIN));
        assert_eq!(interval(&s, None, None, NOW), 1800);
    }

    #[test]
    fn after_a_reset_the_interval_is_the_minimum() {
        let s = snap(NOW, 10.0, Some(NOW + 10 * MIN));
        assert_eq!(interval(&s, None, None, NOW + 10 * MIN), 120);
        assert_eq!(interval(&s, None, None, NOW + 2 * HOUR), 120);
        // The due time of the stored state is at the minimum interval too.
        let mut state = State::default();
        let due = state.update("a", Some(&s), None, NOW + 11 * MIN, &cfg());
        assert_eq!(due, NOW + 2 * MIN);
    }

    #[test]
    fn a_recent_launch_halves_the_interval() {
        let s = snap(NOW, 0.0, None);
        assert_eq!(interval(&s, None, Some(NOW - 10 * MIN), NOW), 900);
    }

    #[test]
    fn the_interval_is_always_within_min_and_max() {
        let usage = config::Usage {
            min_interval_secs: 200,
            max_interval_secs: 400,
            ..cfg()
        };
        for percent in [0.0, 30.0, 60.0, 89.0, 90.0, 100.0, 250.0, -5.0] {
            for reset in [None, Some(NOW + SEC), Some(NOW + 10 * HOUR)] {
                for previous in [
                    None,
                    Some(Point {
                        fetched_at_ms: NOW - SEC,
                        max_percent: 0.0,
                    }),
                ] {
                    for launch in [None, Some(NOW)] {
                        for now in [NOW, NOW + 20 * HOUR] {
                            let s = snap(NOW, percent, reset);
                            let secs = next_interval(&s, previous, launch, now, &usage);
                            assert!((200..=400).contains(&secs), "{percent} {reset:?}: {secs}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn adaptive_false_gives_the_fixed_5_minutes() {
        let usage = config::Usage {
            adaptive: false,
            ..cfg()
        };
        for s in [snap(NOW, 0.0, None), snap(NOW, 95.0, Some(NOW + MIN))] {
            assert_eq!(next_interval(&s, None, Some(NOW), NOW + HOUR, &usage), 300);
        }
        let mut state = State::default();
        let s = snap(NOW, 50.0, None);
        assert_eq!(
            state.update("a", Some(&s), None, NOW, &usage),
            NOW + 5 * MIN
        );
    }

    #[test]
    fn update_keeps_the_fetch_before_and_the_retry_time() {
        let mut state = State::default();
        // No cache: due now.
        assert_eq!(state.update("a", None, None, NOW, &cfg()), 0);
        let first = snap(NOW, 30.0, Some(NOW + 3 * HOUR));
        state.update("a", Some(&first), None, NOW, &cfg());
        assert_eq!(state.accounts["a"].previous, None);
        // The same cache again does not change `previous`.
        state.update("a", Some(&first), None, NOW + MIN, &cfg());
        assert_eq!(state.accounts["a"].previous, None);
        let second = snap(NOW + 5 * MIN, 50.0, Some(NOW + 3 * HOUR));
        let due = state.update("a", Some(&second), None, NOW + 5 * MIN, &cfg());
        let entry = &state.accounts["a"];
        assert_eq!(entry.previous, Some(Point::of(&first)));
        assert_eq!(entry.latest, Some(Point::of(&second)));
        // 20 points in 5 minutes: the rise gives 300 s.
        assert_eq!(entry.interval_secs, Some(300));
        assert_eq!(due, NOW + 10 * MIN);
        assert_eq!(entry.next_due_ms, due);

        // A failed fetch at NOW + 20 min: due again after the minimum.
        state.accounts.get_mut("a").unwrap().last_attempt_ms = Some(NOW + 20 * MIN);
        let due = state.update("a", Some(&second), None, NOW + 20 * MIN, &cfg());
        assert_eq!(due, NOW + 22 * MIN);

        state.update("b", None, None, NOW, &cfg());
        state.retain(&["b"]);
        assert_eq!(state.accounts.keys().collect::<Vec<_>>(), ["b"]);
    }

    #[test]
    fn state_and_launch_files_are_written_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().as_os_str();
        assert_eq!(read_state(home), State::default());
        let mut state = State::default();
        state.update("a", Some(&snap(NOW, 1.0, None)), None, NOW, &cfg());
        write_state(home, &state).unwrap();
        assert_eq!(read_state(home), state);
        fs::write(state_path(home), "{bad").unwrap();
        assert_eq!(read_state(home), State::default());

        assert_eq!(last_launch(home, "a"), None);
        record_launch(home, "a", 42).unwrap();
        record_launch(home, "a", 43).unwrap();
        assert_eq!(last_launch(home, "a"), Some(43));
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(launch_path(home, "a"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

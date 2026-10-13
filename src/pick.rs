//! Automatic selection (#10): the picker, the pick state and the banner.
//!
//! The picker is a pure function. It gets the candidate accounts with their
//! usage, the last account, the session model from the claude arguments,
//! the `[pick]` config and the time. It returns the account to launch.
//! Every account can be used in every directory. There are no pins.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{self, Account, Config, Result};
use crate::usage::{self, Kind, Limit, Snapshot, Source};

/// The percent used that ccx gives an account with unknown usage.
pub const UNKNOWN_PERCENT: f64 = 50.0;

/// One account that the picker can select.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub name: String,
    pub weight: f64,
    /// `None` when there is no usage cache and no `cachedUsageUtilization`.
    pub usage: Option<Snapshot>,
    /// The `model` key in the `settings.json` of the account.
    pub settings_model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reason {
    /// The account with the best score.
    Best,
    /// The last account, because its score is near the best score.
    Sticky,
    /// All accounts are limited. This account has the soonest reset.
    /// `frees_at_ms` is `None` when the reset time is not known.
    AllLimited { frees_at_ms: Option<i64> },
}

/// The result of the picker: an index into the candidates and the reason.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decision {
    pub index: usize,
    pub reason: Reason,
}

/// The default weight of an account. `org_type` is
/// `oauthAccount.organizationType` and `tier` is
/// `oauthAccount.organizationRateLimitTier` from `.claude.json`.
/// Pro 1, Max 5x 5, Max 20x 20. Team, Enterprise and unknown tiers use 5.
pub fn tier_weight(org_type: Option<&str>, tier: Option<&str>) -> f64 {
    match (org_type, tier) {
        (Some("claude_pro"), _) => 1.0,
        (Some("claude_team" | "claude_enterprise"), _) => 5.0,
        (_, Some("default_claude_max_20x")) => 20.0,
        _ => 5.0,
    }
}

/// The `--model` value in the claude arguments (`--model X` or
/// `--model=X`). The search stops at `--`.
pub fn model_arg(args: &[OsString]) -> Option<String> {
    let mut iter = args.iter().map(|a| a.to_string_lossy());
    while let Some(arg) = iter.next() {
        if arg == "--" {
            return None;
        }
        if arg == "--model" {
            return iter.next().map(|v| v.into_owned());
        }
        if let Some(value) = arg.strip_prefix("--model=") {
            return Some(value.to_string());
        }
    }
    None
}

/// True for a print run (`-p` or `--print` before `--`).
pub fn is_print(args: &[OsString]) -> bool {
    args.iter()
        .take_while(|a| *a != "--")
        .any(|a| a == "-p" || a == "--print")
}

/// The percent used of a window. A window that has reset counts as 0%.
fn current(limit: &Limit, now_ms: u64) -> f64 {
    if limit.has_reset(now_ms) {
        0.0
    } else {
        limit.percent
    }
}

/// True when a scoped window applies to the session model: the scope name
/// is a case-insensitive substring of the model name.
fn scope_matches(limit: &Limit, model: Option<&str>) -> bool {
    match (&limit.scope, model) {
        (Some(scope), Some(model)) => model.to_lowercase().contains(&scope.to_lowercase()),
        _ => false,
    }
}

/// The windows that are at or above their thresholds.
fn limiting<'a>(
    usage: &'a Snapshot,
    pick: &config::Pick,
    model: Option<&'a str>,
    now_ms: u64,
) -> impl Iterator<Item = &'a Limit> {
    let (session, weekly, scoped) = (
        f64::from(pick.session_avoid),
        f64::from(pick.weekly_avoid),
        f64::from(pick.scoped_avoid),
    );
    usage.limits.iter().filter(move |l| {
        let avoid = match l.kind {
            Kind::Session => session,
            Kind::WeeklyAll => weekly,
            Kind::WeeklyScoped if scope_matches(l, model) => scoped,
            Kind::WeeklyScoped => return false,
        };
        current(l, now_ms) >= avoid
    })
}

/// True when the account is limited. Unknown usage is never limited. A
/// scoped window counts only when it matches the session model.
pub fn limited(
    usage: Option<&Snapshot>,
    pick: &config::Pick,
    model: Option<&str>,
    now_ms: u64,
) -> bool {
    usage.is_some_and(|u| limiting(u, pick, model, now_ms).next().is_some())
}

/// The windows that limit the account, for example `5h 92%, wk 96%` or
/// `Opus wk 99%`.
pub fn limit_text(
    usage: &Snapshot,
    pick: &config::Pick,
    model: Option<&str>,
    now_ms: u64,
) -> String {
    limiting(usage, pick, model, now_ms)
        .map(|l| {
            let name = match (l.kind, &l.scope) {
                (Kind::Session, _) => "5h".to_string(),
                (Kind::WeeklyAll, _) => "wk".to_string(),
                (Kind::WeeklyScoped, Some(scope)) => format!("{scope} wk"),
                (Kind::WeeklyScoped, None) => "wk".to_string(),
            };
            format!("{name} {:.0}%", current(l, now_ms))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The time when all limiting windows have reset, or `None` when one of
/// them has no reset time.
fn frees_at(
    usage: &Snapshot,
    pick: &config::Pick,
    model: Option<&str>,
    now_ms: u64,
) -> Option<i64> {
    limiting(usage, pick, model, now_ms)
        .map(|l| l.resets_at.as_deref().and_then(usage::parse_rfc3339_ms))
        .try_fold(i64::MIN, |latest, at| Some(latest.max(at?)))
}

/// Remaining capacity × weight. Unknown usage counts as 50% used.
pub fn score(candidate: &Candidate, now_ms: u64) -> f64 {
    let used = match &candidate.usage {
        None => UNKNOWN_PERCENT,
        Some(usage) => [Kind::Session, Kind::WeeklyAll]
            .into_iter()
            .filter_map(|kind| usage.get(kind))
            .map(|l| current(l, now_ms))
            .fold(0.0, f64::max),
    };
    (100.0 - used).max(0.0) * candidate.weight
}

/// Picks an account. Returns `None` only when there are no candidates.
///
/// The best account has the highest score. On a tie, an account with known
/// usage ranks first, then the order of the candidates. The last account
/// stays when it is not limited and its normalized score (score / best
/// score × 100) is within `stickiness` points of 100. When all accounts are
/// limited, the account with the soonest reset wins.
pub fn pick<'a>(
    candidates: &'a [Candidate],
    last: Option<&str>,
    model_arg: Option<&'a str>,
    config: &config::Pick,
    now_ms: u64,
) -> Option<Decision> {
    let model = |c: &'a Candidate| model_arg.or(c.settings_model.as_deref());
    let mut open: Vec<(usize, f64)> = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| !limited(c.usage.as_ref(), config, model(c), now_ms))
        .map(|(i, c)| (i, score(c, now_ms)))
        .collect();

    if open.is_empty() {
        let (index, frees_at_ms) = candidates
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let usage = c.usage.as_ref().expect("a limited account has usage");
                (i, frees_at(usage, config, model(c), now_ms))
            })
            .min_by_key(|&(i, at)| (at.unwrap_or(i64::MAX), i))?;
        return Some(Decision {
            index,
            reason: Reason::AllLimited { frees_at_ms },
        });
    }

    let unknown = |i: usize| candidates[i].usage.is_none();
    open.sort_by(|(a, sa), (b, sb)| {
        sb.total_cmp(sa)
            .then(unknown(*a).cmp(&unknown(*b)))
            .then(a.cmp(b))
    });
    let (best, best_score) = open[0];
    let sticky = last.and_then(|last| {
        let &(index, score) = open.iter().find(|(i, _)| candidates[*i].name == last)?;
        let normalized = if best_score > 0.0 {
            score / best_score * 100.0
        } else {
            100.0
        };
        // A small margin, so that float rounding cannot change a result
        // that is exactly at the limit.
        (index != best && 100.0 - normalized <= f64::from(config.stickiness) + 1e-9)
            .then_some(index)
    });
    Some(match sticky {
        Some(index) => Decision {
            index,
            reason: Reason::Sticky,
        },
        None => Decision {
            index: best,
            reason: Reason::Best,
        },
    })
}

/// The data that ccx reads for one account from its files.
pub fn candidate(account: &Account, home: &OsStr) -> Candidate {
    let claude_json: Option<Value> = fs::read(usage::claude_json_path(account, home))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let text_at = |pointer: &str| {
        claude_json
            .as_ref()
            .and_then(|v| v.pointer(pointer))
            .and_then(Value::as_str)
    };
    let weight = account.weight.unwrap_or_else(|| {
        tier_weight(
            text_at("/oauthAccount/organizationType"),
            text_at("/oauthAccount/organizationRateLimitTier"),
        )
    });
    // The newer of the ccx cache and `cachedUsageUtilization`. The launch
    // uses the age of this data for the blocking fetch (#11).
    let cache = usage::read_cache(&usage::cache_path(home, &account.name));
    let fallback = claude_json.as_ref().and_then(usage::parse_cached_value);
    let usage = match (cache, fallback) {
        (Some(a), Some(b)) => Some(if b.fetched_at_ms > a.fetched_at_ms {
            b
        } else {
            a
        }),
        (a, b) => a.or(b),
    };
    let settings_model = fs::read(account.dir.join("settings.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|v| v.get("model")?.as_str().map(String::from));
    Candidate {
        name: account.name.clone(),
        weight,
        usage,
        settings_model,
    }
}

/// The enabled accounts as candidates, in name order.
pub fn candidates(config: &Config, home: &OsStr) -> Vec<Candidate> {
    config
        .accounts
        .iter()
        .filter(|a| a.enabled)
        .map(|a| candidate(a, home))
        .collect()
}

/// The pick state: `~/.local/state/ccx/state.json`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct State {
    /// The account of the last automatic pick.
    pub last: Option<String>,
    pub picked_at_ms: Option<u64>,
}

pub fn state_path(home: &OsStr) -> PathBuf {
    usage::state_dir(home).join("state.json")
}

/// Reads the state. A missing or bad file gives the default state.
pub fn read_state(path: &Path) -> State {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Writes the state atomically at mode 0600.
pub fn write_state(path: &Path, state: &State) -> Result<()> {
    let text = serde_json::to_string_pretty(state).expect("the state always serializes");
    usage::write_private(path, text.as_bytes())
}

/// A local wall-clock time: day of the week (0 is Sunday), hour, minute.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clock {
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
}

unsafe extern "C" {
    fn tzset();
}

/// The local calendar time of `ms` (milliseconds since the Unix epoch).
pub fn local_tm(ms: i64) -> Option<libc::tm> {
    let secs = libc::time_t::try_from(ms.div_euclid(1000)).ok()?;
    // SAFETY: `tm` is plain data, so zeroed memory is a valid value.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are valid for the call. `tzset` reads `TZ`.
    let result = unsafe {
        tzset();
        libc::localtime_r(&secs, &mut tm)
    };
    (!result.is_null()).then_some(tm)
}

/// The local time of `ms` (milliseconds since the Unix epoch).
pub fn local_clock(ms: i64) -> Option<Clock> {
    let tm = local_tm(ms)?;
    Some(Clock {
        weekday: u8::try_from(tm.tm_wday).ok()?,
        hour: u8::try_from(tm.tm_hour).ok()?,
        minute: u8::try_from(tm.tm_min).ok()?,
    })
}

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// `14:05` when `at_ms` is less than 24 hours after `now_ms`, else
/// `Thu 09:00`. `clock` gives the wall-clock time of an instant.
pub fn time_text(at_ms: i64, now_ms: u64, clock: impl Fn(i64) -> Option<Clock>) -> String {
    let Some(c) = clock(at_ms) else {
        return "?".into();
    };
    let hm = format!("{:02}:{:02}", c.hour, c.minute);
    if at_ms - (now_ms as i64) < 86_400_000 {
        hm
    } else {
        format!("{} {hm}", WEEKDAYS[usize::from(c.weekday) % 7])
    }
}

/// The warning when all accounts are limited, for example
/// `all accounts limited; angel2 frees 14:05 (37m)`.
pub fn all_limited_warning(
    name: &str,
    frees_at_ms: Option<i64>,
    now_ms: u64,
    clock: impl Fn(i64) -> Option<Clock>,
) -> String {
    match frees_at_ms {
        Some(at) => format!(
            "ccx: warning: all accounts limited; {name} frees {} ({})",
            time_text(at, now_ms, clock),
            usage::short_duration(at - now_ms as i64)
        ),
        None => format!("ccx: warning: all accounts limited; {name} reset time is unknown"),
    }
}

/// The banner line, for example
/// `ccx → angel2 · 5h 23% · wk 61% (resets Thu 09:00) · /usage 4m`.
pub fn banner(candidate: &Candidate, now_ms: u64, clock: impl Fn(i64) -> Option<Clock>) -> String {
    let mut parts = vec![format!("ccx → {}", candidate.name)];
    let Some(usage) = &candidate.usage else {
        parts.push("usage unknown".into());
        return parts.join(" · ");
    };
    if let Some(l) = usage.get(Kind::Session) {
        parts.push(format!("5h {:.0}%", current(l, now_ms)));
    }
    if let Some(l) = usage.get(Kind::WeeklyAll) {
        let reset = l
            .resets_at
            .as_deref()
            .and_then(usage::parse_rfc3339_ms)
            .filter(|_| !l.has_reset(now_ms));
        parts.push(match reset {
            Some(at) => format!(
                "wk {:.0}% (resets {})",
                current(l, now_ms),
                time_text(at, now_ms, &clock)
            ),
            None => format!("wk {:.0}%", current(l, now_ms)),
        });
    }
    let source = match usage.source {
        Source::Usage => "/usage",
        Source::Cached => "cached",
    };
    let age = usage::short_duration(now_ms.saturating_sub(usage.fetched_at_ms) as i64);
    parts.push(format!("{source} {age}"));
    parts.join(" · ")
}

/// True when ccx shows the banner and the warnings: stdout and stderr are
/// terminals and the run is not a print run.
pub fn show_messages(stdout_tty: bool, stderr_tty: bool, args: &[OsString]) -> bool {
    stdout_tty && stderr_tty && !is_print(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic values only.
    const NOW: u64 = 1_767_225_600_000; // 2026-01-01T00:00:00Z, a Thursday
    const HOUR: i64 = 3_600_000;

    fn cfg() -> config::Pick {
        config::Pick {
            session_avoid: 90,
            weekly_avoid: 95,
            scoped_avoid: 98,
            stickiness: 10,
        }
    }

    fn utc(ms: i64) -> Option<Clock> {
        let secs = ms.div_euclid(1000);
        let days = secs.div_euclid(86_400);
        let rest = secs.rem_euclid(86_400);
        Some(Clock {
            // 1970-01-01 is a Thursday.
            weekday: ((days + 4).rem_euclid(7)) as u8,
            hour: (rest / 3600) as u8,
            minute: (rest % 3600 / 60) as u8,
        })
    }

    fn rfc(ms: i64) -> String {
        // Only for whole hours from 24 hours before NOW, in the tests below.
        let hours = (ms - NOW as i64) / HOUR;
        if hours < 0 {
            return format!("2025-12-31T{:02}:00:00Z", 24 + hours);
        }
        let (day, hour) = (1 + hours / 24, hours % 24);
        format!("2026-01-{day:02}T{hour:02}:00:00Z")
    }

    fn window(kind: Kind, percent: f64, resets_in_hours: Option<i64>) -> Limit {
        Limit {
            kind,
            percent,
            resets_at: resets_in_hours.map(|h| rfc(NOW as i64 + h * HOUR)),
            scope: None,
        }
    }

    fn scoped(scope: &str, percent: f64) -> Limit {
        Limit {
            scope: Some(scope.into()),
            ..window(Kind::WeeklyScoped, percent, Some(48))
        }
    }

    fn known(name: &str, weight: f64, limits: Vec<Limit>) -> Candidate {
        Candidate {
            name: name.into(),
            weight,
            usage: Some(Snapshot {
                source: Source::Usage,
                fetched_at_ms: NOW - 4 * 60_000,
                limits,
            }),
            settings_model: None,
        }
    }

    fn used(name: &str, weight: f64, session: f64, weekly: f64) -> Candidate {
        known(
            name,
            weight,
            vec![
                window(Kind::Session, session, Some(2)),
                window(Kind::WeeklyAll, weekly, Some(72)),
            ],
        )
    }

    fn unknown(name: &str, weight: f64) -> Candidate {
        Candidate {
            usage: None,
            ..known(name, weight, vec![])
        }
    }

    fn pick_name(cands: &[Candidate], last: Option<&str>, model: Option<&str>) -> String {
        let decision = pick(cands, last, model, &cfg(), NOW).unwrap();
        cands[decision.index].name.clone()
    }

    #[test]
    fn tier_weights() {
        let cases = [
            (Some("claude_pro"), Some("default_claude_ai"), 1.0),
            (Some("claude_max"), Some("default_claude_max_5x"), 5.0),
            (Some("claude_max"), Some("default_claude_max_20x"), 20.0),
            (None, Some("default_claude_max_20x"), 20.0),
            (Some("claude_team"), Some("default_claude_max_20x"), 5.0),
            (Some("claude_enterprise"), None, 5.0),
            (Some("claude_max"), Some("future_tier"), 5.0),
            (None, None, 5.0),
        ];
        for (org, tier, want) in cases {
            assert_eq!(tier_weight(org, tier), want, "{org:?} {tier:?}");
        }
    }

    #[test]
    fn picker_table() {
        struct Case {
            name: &'static str,
            cands: Vec<Candidate>,
            last: Option<&'static str>,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "the weight multiplies the remaining capacity",
                // a: 80 × 1 = 80, b: 30 × 5 = 150, c: 10 × 20 = 200.
                cands: vec![
                    used("a", 1.0, 20.0, 10.0),
                    used("b", 5.0, 70.0, 10.0),
                    used("c", 20.0, 10.0, 90.0),
                ],
                last: None,
                want: "c",
            },
            Case {
                name: "a config weight changes the result",
                cands: vec![used("a", 50.0, 20.0, 10.0), used("c", 20.0, 10.0, 90.0)],
                last: None,
                want: "a",
            },
            Case {
                name: "the highest of session and weekly counts",
                // a: (100 - 60) × 5 = 200, b: (100 - 50) × 5 = 250.
                cands: vec![used("a", 5.0, 10.0, 60.0), used("b", 5.0, 50.0, 0.0)],
                last: None,
                want: "b",
            },
            Case {
                name: "a window that has reset counts as 0%",
                cands: vec![
                    known(
                        "a",
                        5.0,
                        vec![
                            window(Kind::Session, 99.0, Some(-1)),
                            window(Kind::WeeklyAll, 99.0, Some(-1)),
                        ],
                    ),
                    used("b", 5.0, 10.0, 10.0),
                ],
                last: None,
                want: "a",
            },
            Case {
                name: "a null reset time keeps the percent",
                cands: vec![
                    known("a", 5.0, vec![window(Kind::Session, 40.0, None)]),
                    used("b", 5.0, 30.0, 0.0),
                ],
                last: None,
                want: "b",
            },
            Case {
                name: "unknown usage scores 50% used",
                // a: 50 × 5 = 250, b: 60 × 5 = 300.
                cands: vec![unknown("a", 5.0), used("b", 5.0, 40.0, 0.0)],
                last: None,
                want: "b",
            },
            Case {
                name: "unknown usage wins with a higher score",
                cands: vec![unknown("a", 20.0), used("b", 5.0, 0.0, 0.0)],
                last: None,
                want: "a",
            },
            Case {
                name: "on a tie, known usage ranks first",
                cands: vec![unknown("a", 5.0), used("b", 5.0, 50.0, 0.0)],
                last: None,
                want: "b",
            },
            Case {
                name: "on a full tie, the first candidate wins",
                cands: vec![used("a", 5.0, 10.0, 0.0), used("b", 5.0, 10.0, 0.0)],
                last: None,
                want: "a",
            },
            Case {
                name: "a limited account is never picked",
                cands: vec![used("a", 20.0, 90.0, 0.0), used("b", 1.0, 80.0, 80.0)],
                last: None,
                want: "b",
            },
            Case {
                name: "unknown usage is never limited",
                cands: vec![used("a", 20.0, 95.0, 0.0), unknown("b", 1.0)],
                last: None,
                want: "b",
            },
            Case {
                name: "a limited last account does not stay",
                cands: vec![used("a", 5.0, 91.0, 0.0), used("b", 5.0, 50.0, 0.0)],
                last: Some("a"),
                want: "b",
            },
            Case {
                name: "an unknown last account does not stay",
                cands: vec![used("a", 5.0, 0.0, 0.0), used("b", 5.0, 0.0, 0.0)],
                last: Some("gone"),
                want: "a",
            },
        ];
        for case in cases {
            assert_eq!(
                pick_name(&case.cands, case.last, None),
                case.want,
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn session_threshold() {
        for (session, is_limited) in [(89.0, false), (89.99, false), (90.0, true), (100.0, true)] {
            let c = used("a", 5.0, session, 0.0);
            assert_eq!(
                limited(c.usage.as_ref(), &cfg(), None, NOW),
                is_limited,
                "{session}"
            );
        }
    }

    #[test]
    fn weekly_threshold() {
        for (weekly, is_limited) in [(94.0, false), (95.0, true)] {
            let c = used("a", 5.0, 0.0, weekly);
            assert_eq!(
                limited(c.usage.as_ref(), &cfg(), None, NOW),
                is_limited,
                "{weekly}"
            );
        }
    }

    #[test]
    fn scoped_threshold() {
        for (percent, is_limited) in [(97.0, false), (98.0, true)] {
            let c = known("a", 5.0, vec![scoped("Opus", percent)]);
            assert_eq!(
                limited(c.usage.as_ref(), &cfg(), Some("opus"), NOW),
                is_limited,
                "{percent}"
            );
        }
    }

    #[test]
    fn config_values_change_the_thresholds() {
        let config = config::Pick {
            session_avoid: 50,
            weekly_avoid: 60,
            scoped_avoid: 70,
            stickiness: 10,
        };
        let check = |c: Candidate, model, want| {
            assert_eq!(limited(c.usage.as_ref(), &config, model, NOW), want);
        };
        check(used("a", 5.0, 49.0, 0.0), None, false);
        check(used("a", 5.0, 50.0, 0.0), None, true);
        check(used("a", 5.0, 0.0, 59.0), None, false);
        check(used("a", 5.0, 0.0, 60.0), None, true);
        check(
            known("a", 5.0, vec![scoped("Opus", 69.0)]),
            Some("opus"),
            false,
        );
        check(
            known("a", 5.0, vec![scoped("Opus", 70.0)]),
            Some("opus"),
            true,
        );
    }

    #[test]
    fn scoped_match_rules() {
        let full = known("a", 20.0, vec![scoped("Opus", 99.0)]);
        let usage = full.usage.as_ref();
        let c = cfg();
        // Case-insensitive substring of the model name.
        assert!(limited(usage, &c, Some("opus"), NOW));
        assert!(limited(usage, &c, Some("claude-OPUS-4-1"), NOW));
        // No match, or no session model: the scoped row is ignored.
        assert!(!limited(usage, &c, Some("sonnet"), NOW));
        assert!(!limited(usage, &c, Some("op"), NOW));
        assert!(!limited(usage, &c, None, NOW));
        // A scoped window that has reset does not limit.
        let mut old = full.clone();
        old.usage.as_mut().unwrap().limits[0].resets_at = Some(rfc(NOW as i64 - HOUR));
        assert!(!limited(old.usage.as_ref(), &c, Some("opus"), NOW));
    }

    #[test]
    fn scoped_match_uses_the_model_arg_then_settings_model() {
        let mut a = known("a", 20.0, vec![scoped("Opus", 99.0)]);
        let b = used("b", 1.0, 50.0, 0.0);
        // No session model: `a` is not limited and has the best score.
        assert_eq!(pick_name(&[a.clone(), b.clone()], None, None), "a");
        assert_eq!(pick_name(&[a.clone(), b.clone()], None, Some("opus")), "b");
        a.settings_model = Some("Opus".into());
        assert_eq!(pick_name(&[a.clone(), b.clone()], None, None), "b");
        // `--model` wins over the settings model.
        assert_eq!(pick_name(&[a, b], None, Some("sonnet")), "a");
    }

    #[test]
    fn unknown_usage_scores_50_times_the_weight() {
        assert_eq!(score(&unknown("a", 20.0), NOW), 1000.0);
        assert_eq!(score(&unknown("a", 1.0), NOW), 50.0);
        assert!(!limited(None, &cfg(), Some("opus"), NOW));
        // An account with only unknown accounts is picked, never blocked.
        let decision = pick(&[unknown("a", 5.0)], None, None, &cfg(), NOW).unwrap();
        assert_eq!(decision.reason, Reason::Best);
    }

    #[test]
    fn stickiness() {
        // best: 100 × 5 = 500. last: normalized 90 (10 points), then 89.
        let best = used("best", 5.0, 0.0, 0.0);
        let near = used("last", 5.0, 10.0, 0.0);
        let far = used("last", 5.0, 11.0, 0.0);
        let decide = |last: &Candidate, config: &config::Pick| {
            let cands = [best.clone(), last.clone()];
            let d = pick(&cands, Some("last"), None, config, NOW).unwrap();
            (cands[d.index].name.clone(), d.reason)
        };
        assert_eq!(decide(&near, &cfg()), ("last".into(), Reason::Sticky));
        assert_eq!(decide(&far, &cfg()), ("best".into(), Reason::Best));
        let wide = config::Pick {
            stickiness: 20,
            ..cfg()
        };
        assert_eq!(decide(&far, &wide), ("last".into(), Reason::Sticky));
        let off = config::Pick {
            stickiness: 0,
            ..cfg()
        };
        assert_eq!(decide(&near, &off), ("best".into(), Reason::Best));
        // The last account is the best account: the reason is Best.
        let d = pick(std::slice::from_ref(&best), Some("best"), None, &cfg(), NOW).unwrap();
        assert_eq!(d.reason, Reason::Best);
    }

    #[test]
    fn all_limited_picks_the_soonest_reset() {
        let cands = [
            known(
                "a",
                20.0,
                vec![
                    window(Kind::Session, 95.0, Some(1)),
                    window(Kind::WeeklyAll, 96.0, Some(30)),
                ],
            ),
            used("b", 1.0, 92.0, 10.0),
            known("c", 5.0, vec![window(Kind::Session, 99.0, None)]),
        ];
        let d = pick(&cands, Some("a"), None, &cfg(), NOW).unwrap();
        // `a` frees when both of its limiting windows reset (30 hours).
        // `b` frees in 2 hours. `c` has no reset time.
        assert_eq!(cands[d.index].name, "b");
        assert_eq!(
            d.reason,
            Reason::AllLimited {
                frees_at_ms: Some(NOW as i64 + 2 * HOUR)
            }
        );
        // Only unknown reset times: the first candidate.
        let d = pick(&cands[2..], None, None, &cfg(), NOW).unwrap();
        assert_eq!(d.reason, Reason::AllLimited { frees_at_ms: None });
        assert_eq!(pick(&[], None, None, &cfg(), NOW), None);
    }

    #[test]
    fn model_and_print_args() {
        let os = |args: &[&str]| args.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(model_arg(&os(&["--model", "opus"])), Some("opus".into()));
        assert_eq!(
            model_arg(&os(&["-c", "--model=sonnet"])),
            Some("sonnet".into())
        );
        assert_eq!(model_arg(&os(&["--", "--model", "opus"])), None);
        assert_eq!(model_arg(&os(&["--model"])), None);
        assert!(is_print(&os(&["-p", "hi"])));
        assert!(is_print(&os(&["--model", "x", "--print"])));
        assert!(!is_print(&os(&["--", "-p"])));
        assert!(!is_print(&os(&["-c"])));
        assert!(show_messages(true, true, &os(&["-c"])));
        assert!(!show_messages(false, true, &os(&[])));
        assert!(!show_messages(true, false, &os(&[])));
        assert!(!show_messages(true, true, &os(&["-p", "hi"])));
    }

    #[test]
    fn banner_and_warning_text() {
        let c = known(
            "angel2",
            5.0,
            vec![
                window(Kind::Session, 23.4, Some(2)),
                window(Kind::WeeklyAll, 61.0, Some(57)),
            ],
        );
        assert_eq!(
            banner(&c, NOW, utc),
            "ccx → angel2 · 5h 23% · wk 61% (resets Sat 09:00) · /usage 4m"
        );
        let mut cached = c.clone();
        let usage = cached.usage.as_mut().unwrap();
        usage.source = Source::Cached;
        usage.limits[1].resets_at = Some(rfc(NOW as i64 + 9 * HOUR));
        assert_eq!(
            banner(&cached, NOW, utc),
            "ccx → angel2 · 5h 23% · wk 61% (resets 09:00) · cached 4m"
        );
        assert_eq!(
            banner(&unknown("main", 20.0), NOW, utc),
            "ccx → main · usage unknown"
        );
        let at = NOW as i64 + 14 * HOUR + 37 * 60_000;
        assert_eq!(
            all_limited_warning("angel2", Some(at), NOW, utc),
            "ccx: warning: all accounts limited; angel2 frees 14:37 (14h37m)"
        );
        assert_eq!(
            all_limited_warning("angel2", None, NOW, utc),
            "ccx: warning: all accounts limited; angel2 reset time is unknown"
        );
    }

    #[test]
    fn limit_text_names_the_limiting_windows() {
        let c = known(
            "a",
            5.0,
            vec![
                window(Kind::Session, 92.4, Some(2)),
                window(Kind::WeeklyAll, 96.0, Some(72)),
                scoped("Opus", 99.0),
            ],
        );
        let usage = c.usage.as_ref().unwrap();
        assert_eq!(
            limit_text(usage, &cfg(), Some("opus"), NOW),
            "5h 92%, wk 96%, Opus wk 99%"
        );
        assert_eq!(limit_text(usage, &cfg(), None, NOW), "5h 92%, wk 96%");
        let ok = used("b", 5.0, 10.0, 10.0);
        assert_eq!(
            limit_text(ok.usage.as_ref().unwrap(), &cfg(), None, NOW),
            ""
        );
    }

    #[test]
    fn local_clock_is_valid() {
        let c = local_clock(NOW as i64).unwrap();
        assert!(c.weekday < 7 && c.hour < 24 && c.minute < 60);
    }

    #[test]
    fn state_write_is_atomic_at_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = state_path(dir.path().as_os_str());
        assert_eq!(read_state(&path), State::default());
        let state = State {
            last: Some("angel2".into()),
            picked_at_ms: Some(NOW),
        };
        write_state(&path, &state).unwrap();
        assert_eq!(read_state(&path), state);
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["state.json"]);
        fs::write(&path, "not json").unwrap();
        assert_eq!(read_state(&path), State::default());
    }
}

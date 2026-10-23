//! Automatic selection (#10): the picker, the pick state and the banner.
//!
//! ccx fills the accounts in order: it uses the first account until it is
//! limited, then the next one. The order is fixed: the base account, then
//! the other subscription accounts by name, then the API-key accounts.
//! The picker is a pure function. It gets the candidate accounts in that
//! order with their usage, the session model from the claude arguments,
//! the `[pick]` config and the time. It returns the account to launch.
//! Every account can be used in every directory. There are no pins.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{self, Account, Config, Result};
use crate::usage::{self, Kind, Limit, Snapshot, Source};

/// One account that the picker can select.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub name: String,
    /// `None` when there is no usage cache and no `cachedUsageUtilization`.
    pub usage: Option<Snapshot>,
    /// The `model` key in the `settings.json` of the account.
    pub settings_model: Option<String>,
    /// True for an API-key account (#20). The picker never selects it. It
    /// is only a paid fallback when all subscription accounts are limited.
    pub paid: bool,
    /// The number of running Claude Code sessions in the account (ADR
    /// 0013). The banner shows them. They do not change the pick (ADR 0014).
    pub running: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reason {
    /// The first account in the fill order that is not limited.
    First,
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

/// Picks an account. Returns `None` only when there is no subscription
/// candidate.
///
/// `candidates` are in the fill order (refer to [`candidates`]). The pick
/// is the first subscription account that is not limited. Unknown usage
/// never makes an account limited. When all subscription accounts are
/// limited, the account with the soonest reset wins.
pub fn pick<'a>(
    candidates: &'a [Candidate],
    model_arg: Option<&'a str>,
    config: &config::Pick,
    now_ms: u64,
) -> Option<Decision> {
    let model = |c: &'a Candidate| model_arg.or(c.settings_model.as_deref());
    let first = candidates
        .iter()
        .position(|c| !c.paid && !limited(c.usage.as_ref(), config, model(c), now_ms));
    if let Some(index) = first {
        return Some(Decision {
            index,
            reason: Reason::First,
        });
    }
    let (index, frees_at_ms) = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.paid)
        .map(|(i, c)| {
            let usage = c.usage.as_ref().expect("a limited account has usage");
            (i, frees_at(usage, config, model(c), now_ms))
        })
        .min_by_key(|&(i, at)| (at.unwrap_or(i64::MAX), i))?;
    Some(Decision {
        index,
        reason: Reason::AllLimited { frees_at_ms },
    })
}

/// The paid fallback (#20): the first API-key candidate, only when all
/// subscription accounts are limited. ccx must still ask the user before
/// it uses it.
pub fn paid_offer(candidates: &[Candidate], decision: Option<&Decision>) -> Option<usize> {
    if !matches!(decision?.reason, Reason::AllLimited { .. }) {
        return None;
    }
    candidates.iter().position(|c| c.paid)
}

/// Adds a usage-limit hit from a transcript to the usage of the candidate
/// (ADR 0013). The hit fills its window to 100% until the reset. ccx uses
/// the hit only when it is newer than the usage data.
pub fn apply_hit(candidate: &mut Candidate, hit: &crate::limit_hit::Hit) {
    let limit = hit.limit();
    match &mut candidate.usage {
        Some(usage) if usage.fetched_at_ms as i64 >= hit.at_ms => {}
        Some(usage) => {
            usage
                .limits
                .retain(|l| !(l.kind == limit.kind && l.scope == limit.scope));
            usage.limits.push(limit);
        }
        None => {
            candidate.usage = Some(Snapshot {
                source: Source::Transcript,
                fetched_at_ms: u64::try_from(hit.at_ms).unwrap_or(0),
                limits: vec![limit],
            });
        }
    }
}

/// The warning for `ccx @name` on a limited account, for example
/// `ccx: warning: angel2 is limited (5h 100%); it frees 17:30 (1h16m).
/// Run ccx with no @name to pick an account.` `None` when the account is
/// not limited.
pub fn named_warning(
    candidate: &Candidate,
    pick: &config::Pick,
    model: Option<&str>,
    now_ms: u64,
    clock: impl Fn(i64) -> Option<Clock>,
) -> Option<String> {
    let usage = candidate.usage.as_ref()?;
    if candidate.paid || !limited(Some(usage), pick, model, now_ms) {
        return None;
    }
    let frees = match frees_at(usage, pick, model, now_ms) {
        Some(at) => format!(
            "it frees {} ({})",
            time_text(at, now_ms, &clock),
            usage::short_duration(at - now_ms as i64)
        ),
        None => "the reset time is unknown".to_string(),
    };
    Some(format!(
        "ccx: warning: {} is limited ({}); {frees}. Run ccx with no @name to pick an account.",
        candidate.name,
        limit_text(usage, pick, model, now_ms)
    ))
}

/// The data that ccx reads for one account from its files.
pub fn candidate(account: &Account, home: &OsStr) -> Candidate {
    if account.is_paid() {
        // An API-key account has no usage windows and no tier.
        return Candidate {
            name: account.name.clone(),
            usage: None,
            settings_model: None,
            paid: true,
            running: 0,
        };
    }
    let claude_json: Option<Value> = fs::read(usage::claude_json_path(account, home))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
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
        usage,
        settings_model,
        paid: false,
        running: crate::handoff::running_count(&account.dir),
    }
}

/// The enabled accounts in the fill order: the base account first, then
/// the other subscription accounts by name, then the API-key accounts by
/// name.
pub fn fill_order(accounts: &[Account]) -> Vec<&Account> {
    let mut out: Vec<&Account> = accounts.iter().filter(|a| a.enabled).collect();
    // The sort is stable and `accounts` is in name order.
    out.sort_by_key(|a| (!a.is_base, a.is_paid()));
    out
}

/// The enabled accounts as candidates, in the fill order.
pub fn candidates(config: &Config, home: &OsStr) -> Vec<Candidate> {
    fill_order(&config.accounts)
        .into_iter()
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
#[cfg(test)]
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
/// `ccx → angel2 · 5h 23% · wk 61% (resets Thu 09:00) · 1 running · /usage 4m`.
pub fn banner(candidate: &Candidate, now_ms: u64, clock: impl Fn(i64) -> Option<Clock>) -> String {
    if candidate.paid {
        return crate::paid::banner(&candidate.name);
    }
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
    if candidate.running > 0 {
        parts.push(format!("{} running", candidate.running));
    }
    let source = match usage.source {
        Source::Usage => "/usage",
        Source::Cached => "cached",
        Source::Transcript => "transcript",
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

    fn known(name: &str, limits: Vec<Limit>) -> Candidate {
        Candidate {
            name: name.into(),
            usage: Some(Snapshot {
                source: Source::Usage,
                fetched_at_ms: NOW - 4 * 60_000,
                limits,
            }),
            settings_model: None,
            paid: false,
            running: 0,
        }
    }

    fn used(name: &str, session: f64, weekly: f64) -> Candidate {
        known(
            name,
            vec![
                window(Kind::Session, session, Some(2)),
                window(Kind::WeeklyAll, weekly, Some(72)),
            ],
        )
    }

    fn unknown(name: &str) -> Candidate {
        Candidate {
            usage: None,
            ..known(name, vec![])
        }
    }

    fn pick_name(cands: &[Candidate], model: Option<&str>) -> String {
        let decision = pick(cands, model, &cfg(), NOW).unwrap();
        cands[decision.index].name.clone()
    }

    fn paid(name: &str) -> Candidate {
        Candidate {
            paid: true,
            ..unknown(name)
        }
    }

    // API-key accounts (#20).

    #[test]
    fn paid_accounts_are_never_picked_while_a_subscription_account_is_usable() {
        let sets = [
            vec![paid("paid"), used("a", 89.0, 94.0)],
            vec![used("a", 95.0, 10.0), paid("paid"), used("b", 80.0, 10.0)],
            // Unknown usage is never limited, so `a` is usable.
            vec![paid("paid"), unknown("a")],
        ];
        for cands in sets {
            let decision = pick(&cands, None, &cfg(), NOW).unwrap();
            assert!(!cands[decision.index].paid, "{cands:?}");
            assert!(!matches!(decision.reason, Reason::AllLimited { .. }));
            assert_eq!(paid_offer(&cands, Some(&decision)), None, "{cands:?}");
        }
    }

    #[test]
    fn paid_accounts_are_offered_only_when_all_subscription_accounts_are_limited() {
        let cands = vec![
            used("a", 95.0, 10.0),
            paid("paid"),
            paid("paid2"),
            used("b", 10.0, 96.0),
        ];
        let decision = pick(&cands, None, &cfg(), NOW).unwrap();
        // The soonest-reset rule stays the fallback for a "no".
        assert_eq!(cands[decision.index].name, "a");
        assert!(matches!(decision.reason, Reason::AllLimited { .. }));
        assert_eq!(paid_offer(&cands, Some(&decision)), Some(1));
    }

    #[test]
    fn only_paid_accounts_give_no_pick_and_no_offer() {
        let cands = vec![paid("paid")];
        assert_eq!(pick(&cands, None, &cfg(), NOW), None);
        assert_eq!(paid_offer(&cands, None), None);
        // No paid account: nothing to offer.
        let cands = vec![used("a", 95.0, 10.0)];
        let decision = pick(&cands, None, &cfg(), NOW).unwrap();
        assert_eq!(paid_offer(&cands, Some(&decision)), None);
    }

    #[test]
    fn paid_banner_shows_the_kind_and_no_usage() {
        assert_eq!(
            banner(&paid("paid"), NOW, utc),
            "ccx → paid · api-key (paid)"
        );
    }

    #[test]
    fn picker_table() {
        struct Case {
            name: &'static str,
            cands: Vec<Candidate>,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "the first account wins while it is not limited",
                cands: vec![used("main", 89.0, 94.0), used("a", 0.0, 0.0)],
                want: "main",
            },
            Case {
                name: "a limited account passes to the next account",
                cands: vec![
                    used("main", 90.0, 10.0),
                    used("a", 10.0, 95.0),
                    used("b", 80.0, 80.0),
                ],
                want: "b",
            },
            Case {
                name: "a window that has reset counts as 0%",
                cands: vec![
                    known(
                        "main",
                        vec![
                            window(Kind::Session, 99.0, Some(-1)),
                            window(Kind::WeeklyAll, 99.0, Some(-1)),
                        ],
                    ),
                    used("a", 10.0, 10.0),
                ],
                want: "main",
            },
            Case {
                name: "a null reset time keeps the percent",
                cands: vec![
                    known("main", vec![window(Kind::Session, 95.0, None)]),
                    used("a", 30.0, 0.0),
                ],
                want: "a",
            },
            Case {
                name: "unknown usage is never limited",
                cands: vec![unknown("main"), used("a", 0.0, 0.0)],
                want: "main",
            },
            Case {
                name: "less usage on a later account does not matter",
                cands: vec![used("main", 85.0, 90.0), unknown("a")],
                want: "main",
            },
        ];
        for case in cases {
            assert_eq!(pick_name(&case.cands, None), case.want, "{}", case.name);
        }
    }

    fn account(name: &str, dir: &str, enabled: bool, paid: bool) -> Account {
        Account {
            name: name.into(),
            dir: PathBuf::from(dir),
            dir_text: dir.into(),
            aliases: vec![],
            enabled,
            email: None,
            own: vec![],
            is_base: dir == "/home/test/.claude",
            api_key: paid.then_some(config::KeySource::File),
        }
    }

    #[test]
    fn fill_order_is_base_then_by_name_then_paid() {
        // In name order, as the config gives them.
        let accounts = [
            account("aaa", "/home/test/.claude-aaa", true, false),
            account("api", "/home/test/.claude-api", true, true),
            account("main", "/home/test/.claude", true, false),
            account("off", "/home/test/.claude-off", false, false),
            account("zed", "/home/test/.claude-zed", true, false),
        ];
        let names: Vec<_> = fill_order(&accounts)
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        assert_eq!(names, ["main", "aaa", "zed", "api"]);
        // The base account is first whatever its name is.
        let mut renamed = accounts.clone();
        renamed[2].name = "zzz".into();
        assert_eq!(fill_order(&renamed)[0].name, "zzz");
    }

    #[test]
    fn session_threshold() {
        for (session, is_limited) in [(89.0, false), (89.99, false), (90.0, true), (100.0, true)] {
            let c = used("a", session, 0.0);
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
            let c = used("a", 0.0, weekly);
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
            let c = known("a", vec![scoped("Opus", percent)]);
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
        };
        let check = |c: Candidate, model, want| {
            assert_eq!(limited(c.usage.as_ref(), &config, model, NOW), want);
        };
        check(used("a", 49.0, 0.0), None, false);
        check(used("a", 50.0, 0.0), None, true);
        check(used("a", 0.0, 59.0), None, false);
        check(used("a", 0.0, 60.0), None, true);
        check(known("a", vec![scoped("Opus", 69.0)]), Some("opus"), false);
        check(known("a", vec![scoped("Opus", 70.0)]), Some("opus"), true);
    }

    #[test]
    fn scoped_match_rules() {
        let full = known("a", vec![scoped("Opus", 99.0)]);
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
        let mut a = known("a", vec![scoped("Opus", 99.0)]);
        let b = used("b", 50.0, 0.0);
        // No session model: `a` is not limited and is first.
        assert_eq!(pick_name(&[a.clone(), b.clone()], None), "a");
        assert_eq!(pick_name(&[a.clone(), b.clone()], Some("opus")), "b");
        a.settings_model = Some("Opus".into());
        assert_eq!(pick_name(&[a.clone(), b.clone()], None), "b");
        // `--model` wins over the settings model.
        assert_eq!(pick_name(&[a, b], Some("sonnet")), "a");
    }

    #[test]
    fn unknown_usage_is_picked_never_blocked() {
        assert!(!limited(None, &cfg(), Some("opus"), NOW));
        let decision = pick(&[unknown("a")], None, &cfg(), NOW).unwrap();
        assert_eq!(decision.reason, Reason::First);
    }

    #[test]
    fn all_limited_picks_the_soonest_reset() {
        let cands = [
            known(
                "a",
                vec![
                    window(Kind::Session, 95.0, Some(1)),
                    window(Kind::WeeklyAll, 96.0, Some(30)),
                ],
            ),
            used("b", 92.0, 10.0),
            known("c", vec![window(Kind::Session, 99.0, None)]),
        ];
        let d = pick(&cands, None, &cfg(), NOW).unwrap();
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
        let d = pick(&cands[2..], None, &cfg(), NOW).unwrap();
        assert_eq!(d.reason, Reason::AllLimited { frees_at_ms: None });
        assert_eq!(pick(&[], None, &cfg(), NOW), None);
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
            banner(&unknown("main"), NOW, utc),
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
    fn running_sessions_do_not_change_the_fill_order() {
        let mut a = known("main", vec![window(Kind::Session, 40.0, Some(2))]);
        let b = known("angel2", vec![window(Kind::Session, 10.0, Some(2))]);
        a.running = 2;
        let candidates = [a.clone(), b];
        let decision = pick(&candidates, None, &cfg(), NOW).unwrap();
        assert_eq!(candidates[decision.index].name, "main");
        // The banner shows the running sessions.
        assert_eq!(
            banner(&a, NOW, utc),
            "ccx → main · 5h 40% · 2 running · /usage 4m"
        );
    }

    fn hit(at_ms: i64, resets_in_hours: i64) -> crate::limit_hit::Hit {
        crate::limit_hit::Hit {
            uuid: "00000000-0000-4000-8000-000000000001".into(),
            at_ms,
            resets_at_ms: NOW as i64 + resets_in_hours * HOUR,
            limit_type: "five_hour".into(),
        }
    }

    #[test]
    fn a_newer_hit_fills_its_window() {
        let mut c = known(
            "angel2",
            vec![
                window(Kind::Session, 10.0, Some(2)),
                window(Kind::WeeklyAll, 20.0, Some(48)),
            ],
        );
        // The usage data is 4 minutes old. A hit 1 minute ago is newer.
        apply_hit(&mut c, &hit(NOW as i64 - 60_000, 2));
        assert!(limited(c.usage.as_ref(), &cfg(), None, NOW));
        let usage = c.usage.as_ref().unwrap();
        assert_eq!(usage.limits.len(), 2);
        assert_eq!(usage.get(Kind::Session).unwrap().percent, 100.0);
        assert_eq!(usage.get(Kind::WeeklyAll).unwrap().percent, 20.0);
        assert_eq!(limit_text(usage, &cfg(), None, NOW), "5h 100%".to_string());
    }

    #[test]
    fn an_older_hit_does_not_change_the_usage() {
        let mut c = known("angel2", vec![window(Kind::Session, 10.0, Some(2))]);
        let before = c.clone();
        apply_hit(&mut c, &hit(NOW as i64 - 10 * 60_000, 2));
        assert_eq!(c, before);
    }

    #[test]
    fn a_hit_gives_usage_to_an_account_with_no_usage() {
        let mut c = unknown("angel2");
        apply_hit(&mut c, &hit(NOW as i64 - 60_000, 2));
        let usage = c.usage.as_ref().unwrap();
        assert_eq!(usage.source, Source::Transcript);
        assert!(limited(Some(usage), &cfg(), None, NOW));
        assert!(banner(&c, NOW, utc).ends_with(" · transcript 1m"));
    }

    #[test]
    fn named_warning_text() {
        let c = known("angel2", vec![window(Kind::Session, 95.0, Some(2))]);
        assert_eq!(
            named_warning(&c, &cfg(), None, NOW, utc).as_deref(),
            Some(
                "ccx: warning: angel2 is limited (5h 95%); it frees 02:00 (2h00m). \
                 Run ccx with no @name to pick an account."
            )
        );
        let c = known("angel2", vec![window(Kind::Session, 95.0, None)]);
        assert!(
            named_warning(&c, &cfg(), None, NOW, utc)
                .unwrap()
                .contains("(5h 95%); the reset time is unknown.")
        );
        let ok = known("angel2", vec![window(Kind::Session, 50.0, Some(2))]);
        assert_eq!(named_warning(&ok, &cfg(), None, NOW, utc), None);
        assert_eq!(
            named_warning(&unknown("angel2"), &cfg(), None, NOW, utc),
            None
        );
    }

    #[test]
    fn limit_text_names_the_limiting_windows() {
        let c = known(
            "a",
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
        let ok = used("b", 10.0, 10.0);
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

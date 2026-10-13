//! `ccx status [--json]` and `ccx usage refresh [NAME|--all]`.
//!
//! Status shows one row for each account: the kind and tier, the usage
//! windows, the reset times and the usage source with its age. A failed
//! usage lookup shows "unknown". It never makes the whole command fail.

use std::ffi::OsStr;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::{Value, json};

use crate::config::{self, Account, Config, Pick, Result};
use crate::env::Env;
use crate::launch;
use crate::usage::{self, Kind, Limit, Snapshot};

/// The exit code when all logged-in accounts are limited.
pub const EXIT_ALL_LIMITED: u8 = 3;
/// The exit code when no account is logged in.
pub const EXIT_NO_LOGIN: u8 = 4;
/// The minimum timeout of `ccx usage refresh`. A refresh does not block a
/// launch, so it can wait longer than the blocking fetch.
const REFRESH_TIMEOUT: Duration = Duration::from_secs(30);

/// The login data from `claude auth status --json`.
#[derive(Debug, Clone, PartialEq)]
pub struct Auth {
    pub logged_in: bool,
    pub method: Option<String>,
    pub subscription: Option<String>,
}

/// Reads the JSON of `claude auth status`. It never reads the email.
pub fn parse_auth(text: &str) -> Option<Auth> {
    let value: Value = serde_json::from_str(text.trim()).ok()?;
    let text_of = |key: &str| value.get(key).and_then(Value::as_str).map(String::from);
    Some(Auth {
        logged_in: value.get("loggedIn")?.as_bool()?,
        method: text_of("authMethod"),
        subscription: text_of("subscriptionType"),
    })
}

/// The display name of `organizationRateLimitTier`. An unknown value shows
/// as the raw string.
pub fn tier_label(raw: &str) -> String {
    match raw {
        "default_claude_max_20x" => "Max 20x".into(),
        "default_claude_max_5x" => "Max 5x".into(),
        other => other.into(),
    }
}

/// `oauthAccount.organizationRateLimitTier` from the text of `.claude.json`.
pub fn parse_tier(claude_json: &str) -> Option<String> {
    let value: Value = serde_json::from_str(claude_json).ok()?;
    value
        .pointer("/oauthAccount/organizationRateLimitTier")?
        .as_str()
        .map(String::from)
}

/// The status of one account.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub name: String,
    pub email: Option<String>,
    pub base: bool,
    pub enabled: bool,
    /// `None` when `claude auth status` did not give a result.
    pub auth: Option<Auth>,
    /// The raw `organizationRateLimitTier`.
    pub tier: Option<String>,
    /// `None` when both usage sources are missing: the usage is unknown.
    pub usage: Option<Snapshot>,
    /// Why the `/usage` fetch failed, if it ran and failed.
    pub error: Option<String>,
}

impl Row {
    fn logged_in(&self) -> Option<bool> {
        self.auth.as_ref().map(|a| a.logged_in)
    }
}

/// True when a window is at or above its limit in `[pick]`. Status has no
/// session model, so a scoped weekly window never makes an account limited
/// here. Unknown usage is not limited. A window that has reset at `now_ms`
/// is not limited, because its percent is from the last window.
pub fn limited(usage: Option<&Snapshot>, pick: &Pick, now_ms: u64) -> bool {
    let Some(usage) = usage else {
        return false;
    };
    let at_or_above = |kind, avoid: u32| {
        usage
            .get(kind)
            .is_some_and(|l| !l.has_reset(now_ms) && l.percent >= f64::from(avoid))
    };
    at_or_above(Kind::Session, pick.session_avoid)
        || at_or_above(Kind::WeeklyAll, pick.weekly_avoid)
}

/// 0 when at least one enabled, logged-in account is not limited, 3 when
/// all of them are limited, 4 when no enabled account is logged in.
pub fn exit_code(rows: &[Row], pick: &Pick, now_ms: u64) -> u8 {
    let mut usable = rows
        .iter()
        .filter(|r| r.enabled && r.logged_in() == Some(true))
        .peekable();
    if usable.peek().is_none() {
        return EXIT_NO_LOGIN;
    }
    if usable.all(|r| limited(r.usage.as_ref(), pick, now_ms)) {
        EXIT_ALL_LIMITED
    } else {
        0
    }
}

pub fn run(env: &Env, as_json: bool) -> Result<u8> {
    let config = config::load(env)?;
    let home = config::home(env)?;
    let depth = launch::check_depth(env)?;
    let claude = find_claude(&config, env);
    let timeout = Duration::from_secs(config.usage.fetch_timeout_secs);
    let fresh_ms = config.usage.fresh_minutes * 60_000;
    let rows: Vec<Row> = std::thread::scope(|scope| {
        let handles: Vec<_> = config
            .accounts
            .iter()
            .map(|account| {
                let claude = claude.as_deref();
                scope.spawn(move || {
                    let (account_env, _) = launch::build_env(env, account, depth);
                    gather(account, &account_env, home, claude, timeout, fresh_ms)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("a status thread does not panic"))
            .collect()
    });

    if std::io::stderr().is_terminal() {
        if claude.is_none() {
            eprintln!("ccx: warning: cannot find claude; set launch.claude_bin or CCX_CLAUDE_BIN");
        }
        for row in &rows {
            if let Some(error) = &row.error {
                eprintln!("ccx: warning: {}: usage fetch: {error}", row.name);
            }
        }
    }
    let now = usage::now_ms();
    let code = exit_code(&rows, &config.pick, now);
    if as_json {
        println!("{}", to_json(&rows, &config.pick, now, code));
    } else {
        print!("{}", to_table(&rows, now));
    }
    Ok(code)
}

fn find_claude(config: &Config, env: &Env) -> Option<PathBuf> {
    let self_exe = std::env::current_exe().ok();
    launch::find_claude(
        config.launch.claude_bin.as_deref(),
        env,
        self_exe.as_deref(),
    )
}

/// Collects the status of one account. It uses the ccx cache when it is
/// fresh. If not, it runs the `/usage` fetch and writes the cache. If the
/// fetch fails, it uses `cachedUsageUtilization`.
fn gather(
    account: &Account,
    env: &Env,
    home: &OsStr,
    claude: Option<&Path>,
    timeout: Duration,
    fresh_ms: u64,
) -> Row {
    let auth = claude.and_then(|claude| auth_status(claude, env, home, timeout));
    let tier = std::fs::read_to_string(usage::claude_json_path(account, home))
        .ok()
        .and_then(|text| parse_tier(&text));
    let cache = usage::cache_path(home, &account.name);
    let fresh = usage::read_cache(&cache)
        .filter(|s| usage::now_ms().saturating_sub(s.fetched_at_ms) < fresh_ms);
    let logged_out = auth.as_ref().is_some_and(|a| !a.logged_in);
    let (snapshot, error) = match (fresh, claude) {
        (Some(snapshot), _) => (Some(snapshot), None),
        (None, _) if logged_out => (None, None),
        (None, None) => (None, Some("cannot find claude".to_string())),
        (None, Some(claude)) => match usage::fetch(claude, env, home, timeout) {
            Ok(snapshot) => {
                let error = usage::write_cache(&cache, &snapshot).err();
                (Some(snapshot), error)
            }
            Err(error) => (None, Some(error)),
        },
    };
    Row {
        name: account.name.clone(),
        email: account.email.clone(),
        base: account.is_base,
        enabled: account.enabled,
        auth,
        tier,
        usage: snapshot.or_else(|| usage::fallback(account, home)),
        error,
    }
}

/// Runs `claude auth status --json` in the probe dir.
fn auth_status(claude: &Path, env: &Env, home: &OsStr, timeout: Duration) -> Option<Auth> {
    let probe = usage::probe_dir(home);
    usage::make_private_dir(&probe).ok()?;
    let mut command = Command::new(claude);
    command
        .args(["auth", "status", "--json"])
        .current_dir(&probe)
        .env_clear()
        .envs(env.iter());
    // `claude auth status` exits 1 when the account is logged out, but it
    // still prints the JSON.
    let (_, text) = usage::run_with_timeout(command, timeout).ok()?;
    parse_auth(&text)
}

/// `ccx usage refresh NAME` or `ccx usage refresh --all` (all enabled
/// accounts). Returns false when a fetch or a cache write fails.
pub fn refresh(env: &Env, name: Option<&str>) -> Result<bool> {
    let config = config::load(env)?;
    let home = config::home(env)?;
    let depth = launch::check_depth(env)?;
    let accounts: Vec<&Account> = match name {
        Some(name) => vec![config.account(name).ok_or_else(|| {
            format!(
                "unknown account `{name}`; known accounts: {}",
                config.names().join(", ")
            )
        })?],
        None => config.accounts.iter().filter(|a| a.enabled).collect(),
    };
    let claude = find_claude(&config, env)
        .ok_or("cannot find claude; set launch.claude_bin or CCX_CLAUDE_BIN")?;
    let timeout = REFRESH_TIMEOUT.max(Duration::from_secs(config.usage.fetch_timeout_secs));
    let results: Vec<Result<Snapshot>> = std::thread::scope(|scope| {
        let handles: Vec<_> = accounts
            .iter()
            .map(|account| {
                let claude = claude.as_path();
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
            .map(|h| h.join().expect("a refresh thread does not panic"))
            .collect()
    });
    let mut all_ok = true;
    for (account, result) in accounts.iter().zip(results) {
        match result {
            Ok(snapshot) => println!(
                "{}: ok ({} usage windows)",
                account.name,
                snapshot.limits.len()
            ),
            Err(error) => {
                all_ok = false;
                eprintln!("ccx: {}: usage fetch: {error}", account.name);
            }
        }
    }
    Ok(all_ok)
}

fn percent(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}%")
    } else {
        format!("{value:.1}%")
    }
}

/// `9%`, or `9% (2h05m)` with the time until the window resets, or
/// `reset` when the window has reset and its percent is not current.
fn window(limit: &Limit, now_ms: u64) -> String {
    if limit.has_reset(now_ms) {
        return "reset".into();
    }
    let reset = limit.resets_at.as_deref().and_then(usage::parse_rfc3339_ms);
    match reset {
        Some(at) => format!(
            "{} ({})",
            percent(limit.percent),
            usage::short_duration(at - now_ms as i64)
        ),
        None => percent(limit.percent),
    }
}

fn kind_cell(row: &Row) -> String {
    match &row.auth {
        None => "-".into(),
        Some(auth) if !auth.logged_in => "logged out".into(),
        Some(auth) => auth
            .subscription
            .clone()
            .or_else(|| auth.method.clone())
            .unwrap_or_else(|| "-".into()),
    }
}

/// The text table. The time in parentheses is the time until the window
/// resets. AGE is the age of the usage data.
pub fn to_table(rows: &[Row], now_ms: u64) -> String {
    const HEADER: [&str; 9] = [
        "NAME", "EMAIL", "KIND", "TIER", "SESSION", "WEEKLY", "SCOPED", "SOURCE", "AGE",
    ];
    let mut table = vec![HEADER.map(String::from)];
    for row in rows {
        let name = if row.enabled {
            row.name.clone()
        } else {
            format!("{} (off)", row.name)
        };
        let dash = || "-".to_string();
        let (session, weekly, scoped, source, age) = match &row.usage {
            None => ("unknown".into(), "unknown".into(), dash(), dash(), dash()),
            Some(usage) => {
                let cell = |kind| usage.get(kind).map_or_else(dash, |l| window(l, now_ms));
                let scoped: Vec<_> = usage
                    .scoped()
                    .map(|l| {
                        format!(
                            "{} {}",
                            l.scope.as_deref().unwrap_or("scoped"),
                            window(l, now_ms)
                        )
                    })
                    .collect();
                (
                    cell(Kind::Session),
                    cell(Kind::WeeklyAll),
                    if scoped.is_empty() {
                        dash()
                    } else {
                        scoped.join(", ")
                    },
                    usage.source.label().to_string(),
                    usage::short_duration(now_ms.saturating_sub(usage.fetched_at_ms) as i64),
                )
            }
        };
        table.push([
            name,
            row.email.clone().unwrap_or_else(dash),
            kind_cell(row),
            row.tier.as_deref().map_or_else(dash, tier_label),
            session,
            weekly,
            scoped,
            source,
            age,
        ]);
    }
    let mut widths = [0; HEADER.len()];
    for line in &table {
        for (width, cell) in widths.iter_mut().zip(line) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let mut out = String::new();
    for line in &table {
        let cells: Vec<String> = line
            .iter()
            .zip(widths)
            .map(|(cell, width)| format!("{cell:width$}"))
            .collect();
        out.push_str(cells.join("  ").trim_end());
        out.push('\n');
    }
    out
}

/// One window. When the window has reset, `reset` is true and `percent` is
/// `null`, because the current usage is not known.
fn window_json(limit: &Limit, now_ms: u64) -> Value {
    let reset = limit.has_reset(now_ms);
    json!({
        "percent": (!reset).then_some(limit.percent),
        "reset": reset,
        "resets_at": limit.resets_at,
    })
}

/// Stable JSON: the accounts in name order, the keys in alphabetical order.
/// Unknown values are `null`.
pub fn to_json(rows: &[Row], pick: &Pick, now_ms: u64, exit_code: u8) -> String {
    let accounts: Vec<Value> = rows
        .iter()
        .map(|row| {
            let auth = row.auth.as_ref();
            let usage = row.usage.as_ref();
            let window_of = |kind| {
                usage
                    .and_then(|u| u.get(kind))
                    .map_or(Value::Null, |l| window_json(l, now_ms))
            };
            let scoped: Vec<Value> = usage
                .into_iter()
                .flat_map(Snapshot::scoped)
                .map(|l| {
                    let mut value = window_json(l, now_ms);
                    value["model"] = json!(l.scope);
                    value
                })
                .collect();
            json!({
                "name": row.name,
                "email": row.email,
                "base": row.base,
                "enabled": row.enabled,
                "logged_in": auth.map(|a| a.logged_in),
                "auth_method": auth.and_then(|a| a.method.clone()),
                "subscription_type": auth.and_then(|a| a.subscription.clone()),
                "tier": row.tier.as_deref().map(tier_label),
                "tier_raw": row.tier,
                "source": usage.map(|u| u.source.label()),
                "fetched_at_ms": usage.map(|u| u.fetched_at_ms),
                "age_secs": usage.map(|u| now_ms.saturating_sub(u.fetched_at_ms) / 1000),
                "session": window_of(Kind::Session),
                "weekly": window_of(Kind::WeeklyAll),
                "scoped": scoped,
                "limited": limited(usage, pick, now_ms),
                "error": row.error,
            })
        })
        .collect();
    serde_json::to_string_pretty(&json!({ "accounts": accounts, "exit_code": exit_code }))
        .expect("JSON values always serialize")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::Source;

    // Synthetic values only.
    const NOW: u64 = 1_767_225_600_000; // 2026-01-01T00:00:00Z

    fn pick() -> Pick {
        Pick {
            session_avoid: 90,
            weekly_avoid: 95,
            scoped_avoid: 98,
            stickiness: 10,
        }
    }

    fn limit(kind: Kind, percent: f64, resets_at: Option<&str>, scope: Option<&str>) -> Limit {
        Limit {
            kind,
            percent,
            resets_at: resets_at.map(String::from),
            scope: scope.map(String::from),
        }
    }

    fn snapshot(session: f64, weekly: f64) -> Snapshot {
        Snapshot {
            source: Source::Usage,
            fetched_at_ms: NOW - 120_000,
            limits: vec![
                limit(Kind::Session, session, Some("2026-01-01T02:05:00Z"), None),
                limit(Kind::WeeklyAll, weekly, None, None),
                limit(
                    Kind::WeeklyScoped,
                    12.5,
                    Some("2026-01-03T04:00:00.000000+00:00"),
                    Some("ExampleModel"),
                ),
            ],
        }
    }

    fn row(name: &str, logged_in: Option<bool>, usage: Option<Snapshot>) -> Row {
        Row {
            name: name.into(),
            email: None,
            base: false,
            enabled: true,
            auth: logged_in.map(|logged_in| Auth {
                logged_in,
                method: Some("claude.ai".into()),
                subscription: logged_in.then(|| "max".into()),
            }),
            tier: None,
            usage,
            error: None,
        }
    }

    #[test]
    fn auth_status_json_is_parsed_without_the_email() {
        let auth = parse_auth(
            r#"{"loggedIn": true, "authMethod": "claude.ai", "email": "main@example.com",
                "subscriptionType": "max", "newField": 1}"#,
        )
        .unwrap();
        assert_eq!(
            auth,
            Auth {
                logged_in: true,
                method: Some("claude.ai".into()),
                subscription: Some("max".into())
            }
        );
        assert!(!parse_auth(r#"{"loggedIn": false}"#).unwrap().logged_in);
        assert_eq!(parse_auth("not json"), None);
        assert_eq!(parse_auth("{}"), None);
    }

    #[test]
    fn known_tiers_have_names_and_unknown_tiers_show_raw() {
        assert_eq!(tier_label("default_claude_max_20x"), "Max 20x");
        assert_eq!(tier_label("default_claude_max_5x"), "Max 5x");
        assert_eq!(tier_label("future_tier_xyz"), "future_tier_xyz");
        assert_eq!(
            parse_tier(r#"{"oauthAccount": {"organizationRateLimitTier": "future_tier_xyz"}}"#),
            Some("future_tier_xyz".into())
        );
        assert_eq!(parse_tier(r#"{"oauthAccount": {}}"#), None);
        assert_eq!(parse_tier("not json"), None);
    }

    #[test]
    fn limited_uses_session_and_weekly_thresholds() {
        let p = pick();
        assert!(!limited(None, &p, NOW));
        assert!(!limited(Some(&snapshot(89.9, 94.0)), &p, NOW));
        assert!(limited(Some(&snapshot(90.0, 0.0)), &p, NOW));
        assert!(limited(Some(&snapshot(0.0, 95.0)), &p, NOW));
        // A scoped window at 100% does not make the account limited here.
        let mut s = snapshot(0.0, 0.0);
        s.limits[2].percent = 100.0;
        assert!(!limited(Some(&s), &p, NOW));
    }

    /// A snapshot with session 95% and weekly 97%. Each window has a reset
    /// time three hours before NOW, or in the future when it is not `past`.
    fn old_snapshot(session_past: bool, weekly_past: bool) -> Snapshot {
        let at = |past| {
            Some(if past {
                "2025-12-31T21:00:00Z"
            } else {
                "2026-01-01T02:05:00Z"
            })
        };
        Snapshot {
            source: Source::Cached,
            fetched_at_ms: NOW - 6 * 3_600_000,
            limits: vec![
                limit(Kind::Session, 95.0, at(session_past), None),
                limit(Kind::WeeklyAll, 97.0, at(weekly_past), None),
            ],
        }
    }

    #[test]
    fn an_expired_session_window_is_not_limited() {
        let p = pick();
        let mut s = old_snapshot(true, false);
        s.limits[1].percent = 10.0;
        assert!(!limited(Some(&s), &p, NOW));
        let rows = [row("a", Some(true), Some(s))];
        assert_eq!(exit_code(&rows, &p, NOW), 0);
        // The same window is limited before its reset time.
        let before_reset = NOW - 4 * 3_600_000;
        assert!(limited(rows[0].usage.as_ref(), &p, before_reset));
    }

    #[test]
    fn an_expired_weekly_window_is_not_limited() {
        let p = pick();
        let mut s = old_snapshot(false, true);
        s.limits[0].percent = 10.0;
        assert!(!limited(Some(&s), &p, NOW));
        assert_eq!(exit_code(&[row("a", Some(true), Some(s))], &p, NOW), 0);
        // Both windows expired: the account is usable.
        let both = old_snapshot(true, true);
        assert!(!limited(Some(&both), &p, NOW));
        // Only the session window expired: the weekly window still limits.
        assert!(limited(Some(&old_snapshot(true, false)), &p, NOW));
    }

    #[test]
    fn expired_windows_show_as_reset() {
        let mut s = old_snapshot(true, true);
        s.limits.push(limit(
            Kind::WeeklyScoped,
            99.0,
            Some("2025-12-31T21:00:00Z"),
            Some("ExampleModel"),
        ));
        let rows = vec![row("a", Some(true), Some(s))];
        assert_eq!(
            to_table(&rows, NOW),
            "\
NAME  EMAIL  KIND  TIER  SESSION  WEEKLY  SCOPED              SOURCE                  AGE
a     -      max   -     reset    reset   ExampleModel reset  cachedUsageUtilization  6h00m
"
        );

        let value: Value = serde_json::from_str(&to_json(&rows, &pick(), NOW, 0)).unwrap();
        let account = &value["accounts"][0];
        assert_eq!(
            account["session"],
            json!({"percent": null, "reset": true, "resets_at": "2025-12-31T21:00:00Z"})
        );
        assert_eq!(account["weekly"]["reset"], true);
        assert_eq!(account["weekly"]["percent"], Value::Null);
        assert_eq!(account["scoped"][0]["reset"], true);
        assert_eq!(account["limited"], false);
    }

    #[test]
    fn exit_codes() {
        let p = pick();
        let ok = || Some(snapshot(1.0, 1.0));
        let full = || Some(snapshot(99.0, 1.0));
        assert_eq!(
            exit_code(
                &[row("a", Some(true), ok()), row("b", Some(true), full())],
                &p,
                NOW
            ),
            0
        );
        assert_eq!(
            exit_code(
                &[row("a", Some(true), full()), row("b", Some(false), ok())],
                &p,
                NOW
            ),
            3
        );
        // Unknown usage is usable.
        assert_eq!(exit_code(&[row("a", Some(true), None)], &p, NOW), 0);
        assert_eq!(
            exit_code(
                &[row("a", Some(false), ok()), row("b", None, ok())],
                &p,
                NOW
            ),
            4
        );
        assert_eq!(exit_code(&[], &p, NOW), 4);
        // A disabled account does not count.
        let mut off = row("a", Some(true), ok());
        off.enabled = false;
        assert_eq!(exit_code(&[off, row("b", Some(true), full())], &p, NOW), 3);
    }

    fn rows() -> Vec<Row> {
        let mut main = row("main", Some(true), Some(snapshot(9.0, 20.5)));
        main.base = true;
        main.email = Some("main@example.com".into());
        main.tier = Some("default_claude_max_20x".into());
        let mut work = row("work", Some(false), None);
        work.tier = Some("future_tier_xyz".into());
        work.error = Some("timed out after 5s".into());
        let mut old = row(
            "old",
            None,
            Some(Snapshot {
                source: Source::Cached,
                fetched_at_ms: NOW - 3 * 3_600_000,
                limits: vec![],
            }),
        );
        old.enabled = false;
        vec![main, old, work]
    }

    #[test]
    fn table_shows_windows_resets_source_and_age() {
        assert_eq!(
            to_table(&rows(), NOW),
            "\
NAME       EMAIL             KIND        TIER             SESSION     WEEKLY   SCOPED                     SOURCE                  AGE
main       main@example.com  max         Max 20x          9% (2h05m)  20.5%    ExampleModel 12.5% (2d4h)  /usage                  2m
old (off)  -                 -           -                -           -        -                          cachedUsageUtilization  3h00m
work       -                 logged out  future_tier_xyz  unknown     unknown  -                          -                       -
"
        );
    }

    #[test]
    fn json_is_stable() {
        let text = to_json(&rows(), &pick(), NOW, 0);
        assert_eq!(text, to_json(&rows(), &pick(), NOW, 0));
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["exit_code"], 0);
        assert_eq!(
            value["accounts"][0],
            json!({
                "name": "main", "email": "main@example.com", "base": true, "enabled": true,
                "logged_in": true, "auth_method": "claude.ai", "subscription_type": "max",
                "tier": "Max 20x", "tier_raw": "default_claude_max_20x",
                "source": "/usage", "fetched_at_ms": NOW - 120_000, "age_secs": 120,
                "session": {"percent": 9.0, "reset": false, "resets_at": "2026-01-01T02:05:00Z"},
                "weekly": {"percent": 20.5, "reset": false, "resets_at": null},
                "scoped": [{"model": "ExampleModel", "percent": 12.5, "reset": false,
                            "resets_at": "2026-01-03T04:00:00.000000+00:00"}],
                "limited": false, "error": null,
            })
        );
        let work = &value["accounts"][2];
        assert_eq!(work["source"], Value::Null);
        assert_eq!(work["session"], Value::Null);
        assert_eq!(work["tier"], "future_tier_xyz");
        assert_eq!(work["error"], "timed out after 5s");
    }
}

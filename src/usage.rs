//! The `/usage` fetch, its parser, the usage cache and the fallback.
//!
//! ccx gets usage with `claude -p "/usage"` (ADR 0004, ADR 0005). It runs
//! the command in a fixed probe directory with hooks off, so that project
//! settings, hooks, MCP servers and `CLAUDE.md` files have no effect. ccx
//! never reads, keeps or refreshes credentials (ADR 0002).

use std::ffi::OsStr;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{Account, Result};
use crate::env::Env;

/// The arguments of the usage fetch (ADR 0005). `--settings` turns off all
/// hooks. Do not add `--bare`: it can skip the OAuth login.
pub const ARGS: [&str; 9] = [
    "-p",
    "/usage",
    "--output-format",
    "stream-json",
    "--verbose",
    "--no-session-persistence",
    "--strict-mcp-config",
    "--settings",
    r#"{"disableAllHooks":true}"#,
];

/// The state directory of ccx, relative to HOME.
const STATE_DIR: &str = ".local/state/ccx";

/// The kind of a usage window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Session,
    WeeklyAll,
    WeeklyScoped,
}

impl Kind {
    fn parse(text: &str) -> Option<Kind> {
        match text {
            "session" => Some(Kind::Session),
            "weekly_all" => Some(Kind::WeeklyAll),
            "weekly_scoped" => Some(Kind::WeeklyScoped),
            _ => None,
        }
    }
}

/// One usage window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Limit {
    pub kind: Kind,
    /// Percent used, from 0 to 100.
    pub percent: f64,
    /// RFC 3339 text. `None` when the window has not started.
    pub resets_at: Option<String>,
    /// The model name of a scoped window (`scope.model.display_name`).
    pub scope: Option<String>,
}

/// Where the usage data came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    #[serde(rename = "/usage")]
    Usage,
    #[serde(rename = "cachedUsageUtilization")]
    Cached,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Usage => "/usage",
            Source::Cached => "cachedUsageUtilization",
        }
    }
}

/// The usage of one account at one time. This is also the format of the
/// cache file `~/.local/state/ccx/usage/<name>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub source: Source,
    pub fetched_at_ms: u64,
    pub limits: Vec<Limit>,
}

impl Snapshot {
    /// The first window of `kind`.
    pub fn get(&self, kind: Kind) -> Option<&Limit> {
        self.limits.iter().find(|l| l.kind == kind)
    }

    pub fn scoped(&self) -> impl Iterator<Item = &Limit> {
        self.limits.iter().filter(|l| l.kind == Kind::WeeklyScoped)
    }
}

/// Reads `limits[]`. It skips items with an unknown `kind` or with no
/// numeric `percent`, and it ignores unknown fields.
pub fn parse_limits(value: &Value) -> Vec<Limit> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            Some(Limit {
                kind: Kind::parse(item.get("kind")?.as_str()?)?,
                percent: item.get("percent")?.as_f64()?,
                resets_at: item
                    .get("resets_at")
                    .and_then(Value::as_str)
                    .map(String::from),
                scope: item
                    .pointer("/scope/model/display_name")
                    .and_then(Value::as_str)
                    .map(String::from),
            })
        })
        .collect()
}

/// Reads the `stream-json` output of the usage fetch. The data is in
/// `usage_report.rate_limits.limits[]` at the top level of the first
/// `assistant` event (ADR 0005). Lines that are not JSON are skipped.
pub fn parse_stream(text: &str) -> Result<Vec<Limit>> {
    for line in text.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if event.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        return match event.pointer("/usage_report/rate_limits/limits") {
            Some(limits) if limits.is_array() => Ok(parse_limits(limits)),
            _ => Err("the assistant event has no usage_report.rate_limits.limits".into()),
        };
    }
    Err("the output has no assistant event".into())
}

/// Reads `cachedUsageUtilization` from the text of an account's
/// `.claude.json`. It uses `utilization.limits[]` if it is present, else
/// `utilization.five_hour` and `utilization.seven_day`.
pub fn parse_cached(claude_json: &str) -> Option<Snapshot> {
    let root: Value = serde_json::from_str(claude_json).ok()?;
    let cached = root.get("cachedUsageUtilization")?;
    let fetched_at_ms = cached.get("fetchedAtMs")?.as_f64()? as u64;
    let utilization = cached.get("utilization")?;
    let limits = match utilization.get("limits") {
        Some(limits) if limits.is_array() => parse_limits(limits),
        _ => [("five_hour", Kind::Session), ("seven_day", Kind::WeeklyAll)]
            .into_iter()
            .filter_map(|(key, kind)| {
                let window = utilization.get(key)?;
                Some(Limit {
                    kind,
                    percent: window.get("utilization")?.as_f64()?,
                    resets_at: window
                        .get("resets_at")
                        .and_then(Value::as_str)
                        .map(String::from),
                    scope: None,
                })
            })
            .collect(),
    };
    Some(Snapshot {
        source: Source::Cached,
        fetched_at_ms,
        limits,
    })
}

/// The `.claude.json` of an account. The base account keeps it in HOME.
pub fn claude_json_path(account: &Account, home: &OsStr) -> PathBuf {
    if account.is_base {
        Path::new(home).join(".claude.json")
    } else {
        account.dir.join(".claude.json")
    }
}

/// The fallback: `cachedUsageUtilization` in the account's `.claude.json`.
pub fn fallback(account: &Account, home: &OsStr) -> Option<Snapshot> {
    let text = fs::read_to_string(claude_json_path(account, home)).ok()?;
    parse_cached(&text)
}

/// `~/.local/state/ccx/probe`: the fixed cwd of the usage fetch.
pub fn probe_dir(home: &OsStr) -> PathBuf {
    Path::new(home).join(STATE_DIR).join("probe")
}

/// `~/.local/state/ccx/usage/<name>.json`.
pub fn cache_path(home: &OsStr, name: &str) -> PathBuf {
    Path::new(home)
        .join(STATE_DIR)
        .join("usage")
        .join(format!("{name}.json"))
}

/// Makes `dir` and its missing parents at mode 0700.
pub fn make_private_dir(dir: &Path) -> std::io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

pub fn read_cache(path: &Path) -> Option<Snapshot> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Writes the cache file atomically: a temp file at mode 0600 in the same
/// directory, then a rename.
pub fn write_cache(path: &Path, snapshot: &Snapshot) -> Result<()> {
    let error = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
    let dir = path.parent().expect("a cache path has a parent");
    make_private_dir(dir).map_err(error)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let text = serde_json::to_string_pretty(snapshot).expect("a snapshot always serializes");
    let written = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written.map_err(error)
}

/// Runs `command` with stdout piped and stdin and stderr closed. It kills
/// the process after `timeout`. Returns the exit success and the stdout.
pub fn run_with_timeout(mut command: Command, timeout: Duration) -> Result<(bool, String)> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run claude: {e}"))?;
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        stdout.read_to_string(&mut text).map(|_| text)
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("timed out after {}s", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(format!("cannot wait for claude: {e}")),
        }
    };
    let text = reader
        .join()
        .map_err(|_| "cannot read the claude output".to_string())?
        .map_err(|e| format!("cannot read the claude output: {e}"))?;
    Ok((status.success(), text))
}

/// Runs the usage fetch for one account. `env` is the account environment
/// from `launch::build_env`. The cwd is the probe dir, which this function
/// makes at mode 0700 if it does not exist.
pub fn fetch(claude: &Path, env: &Env, home: &OsStr, timeout: Duration) -> Result<Snapshot> {
    let probe = probe_dir(home);
    make_private_dir(&probe).map_err(|e| format!("cannot make {}: {e}", probe.display()))?;
    let mut command = Command::new(claude);
    command
        .args(ARGS)
        .current_dir(&probe)
        .env_clear()
        .envs(env.iter());
    let (success, text) = run_with_timeout(command, timeout)?;
    if !success {
        return Err("claude -p /usage failed".into());
    }
    Ok(Snapshot {
        source: Source::Usage,
        fetched_at_ms: now_ms(),
        limits: parse_stream(&text)?,
    })
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Parses RFC 3339 text, for example `2026-01-04T06:00:00.000000+00:00`,
/// to milliseconds since the Unix epoch.
pub fn parse_rfc3339_ms(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (year, month, day) = (
        digits(text, 0..4)?,
        digits(text, 5..7)?,
        digits(text, 8..10)?,
    );
    let (hour, minute) = (digits(text, 11..13)?, digits(text, 14..16)?);
    let second = digits(text, 17..19)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut rest = &text[19..];
    let mut millis = 0;
    if let Some(fraction) = rest.strip_prefix('.') {
        let count = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if count == 0 {
            return None;
        }
        millis = format!("{:0<3}", &fraction[..count.min(3)])
            .parse::<i64>()
            .ok()?;
        rest = &fraction[count..];
    }
    let offset_secs = match rest.as_bytes() {
        b"Z" | b"z" => 0,
        [sign @ (b'+' | b'-'), _, _, b':', _, _] => {
            let secs = digits(rest, 1..3)? * 3600 + digits(rest, 4..6)? * 60;
            if *sign == b'-' { -secs } else { secs }
        }
        _ => return None,
    };
    let days = days_from_civil(year, month, day);
    let secs = days * 86_400 + hour * 3600 + minute * 60 + second - offset_secs;
    Some(secs * 1000 + millis)
}

/// The number in `text[range]`, which must be ASCII digits only.
fn digits(text: &str, range: std::ops::Range<usize>) -> Option<i64> {
    let part = text.get(range)?;
    if part.bytes().all(|c| c.is_ascii_digit()) {
        part.parse().ok()
    } else {
        None
    }
}

/// Days since 1970-01-01 for a date in the proleptic Gregorian calendar.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// A short duration: `45s`, `12m`, `3h05m` or `2d4h`.
pub fn short_duration(ms: i64) -> String {
    let secs = ms.max(0) / 1000;
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86_400 => format!("{}h{:02}m", secs / 3600, secs % 3600 / 60),
        _ => format!("{}d{}h", secs / 86_400, secs % 86_400 / 3600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;

    // Synthetic fixtures only. The values are examples.

    fn assistant(limits: Value) -> String {
        json!({
            "type": "assistant",
            "session_id": "00000000-0000-4000-8000-000000000000",
            "message": {"role": "assistant", "model": "<synthetic>", "content": []},
            "usage_report": {"rate_limits": {"limits": limits, "extra_usage": {}}, "session": {}},
            "new_top_level_field": 1
        })
        .to_string()
    }

    fn stream(limits: Value) -> String {
        [
            json!({"type": "system", "subtype": "init"}).to_string(),
            assistant(limits),
            json!({"type": "result", "subtype": "success"}).to_string(),
        ]
        .join("\n")
    }

    fn three_kinds() -> Value {
        json!([
            {"kind": "session", "group": "session", "percent": 5, "resets_at": null,
             "severity": "normal", "is_active": false, "scope": null},
            {"kind": "weekly_all", "group": "weekly", "percent": 11.5,
             "resets_at": "2026-01-04T06:00:00.000000+00:00", "scope": null, "extra": [1, 2]},
            {"kind": "weekly_scoped", "group": "weekly", "percent": 0,
             "resets_at": "2026-01-04T06:00:00.000000+00:00",
             "scope": {"model": {"display_name": "ExampleModel"}, "surface": null}}
        ])
    }

    #[test]
    fn parses_all_three_kinds_with_null_reset_and_unknown_fields() {
        let limits = parse_stream(&stream(three_kinds())).unwrap();
        assert_eq!(
            limits,
            vec![
                Limit {
                    kind: Kind::Session,
                    percent: 5.0,
                    resets_at: None,
                    scope: None
                },
                Limit {
                    kind: Kind::WeeklyAll,
                    percent: 11.5,
                    resets_at: Some("2026-01-04T06:00:00.000000+00:00".into()),
                    scope: None
                },
                Limit {
                    kind: Kind::WeeklyScoped,
                    percent: 0.0,
                    resets_at: Some("2026-01-04T06:00:00.000000+00:00".into()),
                    scope: Some("ExampleModel".into())
                },
            ]
        );
    }

    #[test]
    fn skips_unknown_kinds_and_bad_items() {
        let limits = parse_stream(&stream(json!([
            {"kind": "monthly_future", "percent": 1},
            {"kind": "session", "percent": "a lot"},
            {"percent": 3},
            "not an object",
            {"kind": "weekly_all", "percent": 7}
        ])))
        .unwrap();
        assert_eq!(limits.len(), 1);
        assert_eq!(limits[0].kind, Kind::WeeklyAll);
    }

    #[test]
    fn empty_array_gives_no_limits() {
        assert_eq!(parse_stream(&stream(json!([]))).unwrap(), vec![]);
    }

    #[test]
    fn bad_json_and_missing_report_are_errors() {
        assert!(parse_stream("").is_err());
        assert!(parse_stream("{not json\n[1,2").is_err());
        let no_report = json!({"type": "assistant", "message": {}}).to_string();
        assert!(parse_stream(&no_report).is_err());
        // The report must be at the top level, not in `message`.
        let in_message = json!({"type": "assistant", "message": {"usage_report":
            {"rate_limits": {"limits": []}}}})
        .to_string();
        assert!(parse_stream(&in_message).is_err());
        // A bad line before the event is skipped.
        let text = format!("garbage\n{}", assistant(three_kinds()));
        assert_eq!(parse_stream(&text).unwrap().len(), 3);
    }

    #[test]
    fn cached_uses_limits_then_old_windows() {
        let text = json!({
            "other": true,
            "cachedUsageUtilization": {
                "fetchedAtMs": 1_767_225_600_000u64,
                "accountUuid": "00000000-0000-4000-8000-000000000000",
                "utilization": {
                    "five_hour": {"utilization": 9, "resets_at": "2026-01-01T05:00:00Z"},
                    "limits": three_kinds()
                }
            }
        })
        .to_string();
        let snapshot = parse_cached(&text).unwrap();
        assert_eq!(snapshot.source, Source::Cached);
        assert_eq!(snapshot.fetched_at_ms, 1_767_225_600_000);
        assert_eq!(snapshot.limits.len(), 3);

        let old = json!({"cachedUsageUtilization": {"fetchedAtMs": 5, "utilization": {
            "five_hour": {"utilization": 9, "resets_at": "2026-01-01T05:00:00Z"},
            "seven_day": {"utilization": 20, "resets_at": null}
        }}})
        .to_string();
        let snapshot = parse_cached(&old).unwrap();
        assert_eq!(snapshot.get(Kind::Session).unwrap().percent, 9.0);
        assert_eq!(snapshot.get(Kind::WeeklyAll).unwrap().percent, 20.0);
        assert_eq!(snapshot.get(Kind::WeeklyAll).unwrap().resets_at, None);
    }

    #[test]
    fn cached_is_none_when_missing_or_bad() {
        assert_eq!(parse_cached("{}"), None);
        assert_eq!(parse_cached("not json"), None);
        assert_eq!(
            parse_cached(r#"{"cachedUsageUtilization": {"utilization": {}}}"#),
            None
        );
    }

    #[test]
    fn cache_write_is_atomic_at_mode_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_path(dir.path().as_os_str(), "angel2");
        let snapshot = Snapshot {
            source: Source::Usage,
            fetched_at_ms: 42,
            limits: parse_stream(&stream(three_kinds())).unwrap(),
        };
        write_cache(&path, &snapshot).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let usage_dir = fs::metadata(path.parent().unwrap()).unwrap();
        assert_eq!(usage_dir.permissions().mode() & 0o777, 0o700);
        assert_eq!(read_cache(&path), Some(snapshot.clone()));
        // A second write replaces the file and leaves no temp file.
        write_cache(&path, &snapshot).unwrap();
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["angel2.json"]);
    }

    #[test]
    fn cache_file_format_is_stable() {
        let snapshot = Snapshot {
            source: Source::Cached,
            fetched_at_ms: 1,
            limits: vec![Limit {
                kind: Kind::WeeklyScoped,
                percent: 2.0,
                resets_at: None,
                scope: Some("ExampleModel".into()),
            }],
        };
        assert_eq!(
            serde_json::to_value(&snapshot).unwrap(),
            json!({"source": "cachedUsageUtilization", "fetched_at_ms": 1, "limits": [
                {"kind": "weekly_scoped", "percent": 2.0, "resets_at": null, "scope": "ExampleModel"}
            ]})
        );
    }

    #[test]
    fn rfc3339_parses_offsets_and_fractions() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339_ms("2026-01-01T00:00:00Z"),
            Some(1_767_225_600_000)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-01-01T05:00:00.123456+00:00"),
            Some(1_767_225_600_000 + 5 * 3_600_000 + 123)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-01-01T07:00:00+02:00"),
            parse_rfc3339_ms("2026-01-01T05:00:00Z")
        );
        assert_eq!(
            parse_rfc3339_ms("2024-02-29T00:00:00-01:30"),
            Some(1_709_164_800_000 + 90 * 60_000)
        );
        for bad in [
            "",
            "soon",
            "2026-13-01T00:00:00Z",
            "2026-01-01T00:00:00",
            "2026-01-01T00:00:00.Z",
        ] {
            assert_eq!(parse_rfc3339_ms(bad), None, "{bad}");
        }
    }

    #[test]
    fn short_durations() {
        assert_eq!(short_duration(-5), "0s");
        assert_eq!(short_duration(59_999), "59s");
        assert_eq!(short_duration(12 * 60_000), "12m");
        assert_eq!(short_duration(3 * 3_600_000 + 5 * 60_000), "3h05m");
        assert_eq!(short_duration(2 * 86_400_000 + 4 * 3_600_000), "2d4h");
    }

    #[test]
    fn args_turn_off_hooks_and_never_use_bare() {
        assert!(ARGS.contains(&"--no-session-persistence"));
        assert!(ARGS.contains(&"--strict-mcp-config"));
        assert_eq!(ARGS[7..], ["--settings", r#"{"disableAllHooks":true}"#]);
        assert!(!ARGS.contains(&"--bare"));
    }
}

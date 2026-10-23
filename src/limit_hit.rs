//! A usage-limit hit at the end of a session transcript (ADR 0013).
//!
//! When Claude Code stops a turn at a usage limit, it writes an assistant
//! line with `"error": "rate_limit"` and a `quotaLimits` object with
//! `rateLimitType` and `resetsAt` (seconds since the Unix epoch). When this
//! line is the last message of the newest session, it is newer data than
//! the usage cache. ccx then counts the account as limited until the reset.
//!
//! ccx reads only the end of the transcript, so that the launch stays fast.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

use crate::config::Config;
use crate::handoff::{self, Thread};
use crate::usage::{self, Kind, Limit};

/// ccx reads this number of bytes at the end of a transcript.
const TAIL: u64 = 256 * 1024;

/// A rate limit type with no known window, and a reset within this time,
/// counts as the session window.
const SESSION_MS: i64 = 6 * 3_600_000;

/// One usage-limit hit.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// The `uuid` of the transcript line. A handoff copies the line with
    /// the same `uuid`.
    pub uuid: String,
    /// The time of the line.
    pub at_ms: i64,
    /// The time when the limit resets.
    pub resets_at_ms: i64,
    /// `rateLimitType`, for example `five_hour` or `seven_day`.
    pub limit_type: String,
}

impl Hit {
    /// The usage window that the hit fills: 100% used until the reset.
    pub fn limit(&self) -> Limit {
        let (kind, scope) = match self.limit_type.as_str() {
            "five_hour" => (Kind::Session, None),
            "seven_day" => (Kind::WeeklyAll, None),
            "seven_day_opus" => (Kind::WeeklyScoped, Some("Opus".to_string())),
            "seven_day_sonnet" => (Kind::WeeklyScoped, Some("Sonnet".to_string())),
            _ if self.resets_at_ms - self.at_ms <= SESSION_MS => (Kind::Session, None),
            _ => (Kind::WeeklyAll, None),
        };
        Limit {
            kind,
            percent: 100.0,
            resets_at: Some(usage::format_rfc3339(self.resets_at_ms)),
            scope,
        }
    }

    /// True when the limit has not reset at `now_ms`.
    pub fn is_active(&self, now_ms: u64) -> bool {
        self.resets_at_ms > now_ms as i64
    }
}

/// The hit at the end of the transcript `path`, or `None` when the last
/// message is not a usage-limit error.
pub fn read(path: &Path) -> Option<Hit> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(TAIL).read_to_end(&mut bytes).ok()?;
    parse_tail(&bytes, start > 0)
}

/// The hit in the last message of `bytes`. When `cut` is true, the first
/// line can be a part of a line, so ccx skips it. Lines that are not a
/// message (for example `system`, `last-prompt` or `cost-state`) and the
/// lines of a subagent do not stop the search.
pub fn parse_tail(bytes: &[u8], cut: bool) -> Option<Hit> {
    let text = String::from_utf8_lossy(bytes);
    let lines = text.lines().skip(usize::from(cut));
    let last = lines
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|value| {
            matches!(
                value.get("type").and_then(Value::as_str),
                Some("assistant" | "user")
            ) && value.get("isSidechain").and_then(Value::as_bool) != Some(true)
        })?;
    hit(&last)
}

/// The hit in one assistant line.
fn hit(value: &Value) -> Option<Hit> {
    if value.get("type").and_then(Value::as_str) != Some("assistant")
        || value.get("error").and_then(Value::as_str) != Some("rate_limit")
    {
        return None;
    }
    let info = value
        .get("quotaLimits")
        .or_else(|| value.pointer("/apiErrorParams/rate_limit_info"))?;
    let resets_at_secs = info.get("resetsAt").and_then(Value::as_i64)?;
    Some(Hit {
        uuid: value.get("uuid").and_then(Value::as_str)?.to_string(),
        at_ms: usage::parse_rfc3339_ms(value.get("timestamp").and_then(Value::as_str)?)?,
        resets_at_ms: resets_at_secs.checked_mul(1000)?,
        limit_type: info
            .get("rateLimitType")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

/// The note after `ccx` with no arguments, when the newest session in the
/// cwd stopped at the usage limit of `account` and ccx picked a different
/// account.
pub fn continue_note(account: &str) -> String {
    format!(
        "ccx: the last session here stopped at the {account} usage limit. \
         To continue it on a different account, run ccx -c."
    )
}

/// The active hit at the end of `thread`, when it belongs to the account
/// of `thread`. A handoff copies a transcript with its hit line. When a
/// transcript of a different account has the same line, ccx cannot know
/// which account hit the limit, so it ignores the hit.
pub fn find(config: &Config, thread: &Thread, now_ms: u64) -> Option<Hit> {
    let hit = read(&thread.path).filter(|h| h.is_active(now_ms))?;
    let enc = thread.path.parent()?.file_name()?.to_str()?;
    let copied = config
        .accounts
        .iter()
        .filter(|account| account.name != thread.account)
        .flat_map(|account| handoff::transcripts(account, Some(enc)))
        .any(|other| has_line(&other.path, &hit.uuid));
    (!copied).then_some(hit)
}

/// True when the end of the transcript `path` has the line `uuid`.
fn has_line(path: &Path, uuid: &str) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    if file
        .seek(SeekFrom::Start(len.saturating_sub(TAIL)))
        .is_err()
    {
        return false;
    }
    let mut bytes = Vec::new();
    if file.take(TAIL).read_to_end(&mut bytes).is_err() {
        return false;
    }
    let needle = format!("\"uuid\":\"{uuid}\"");
    String::from_utf8_lossy(&bytes).contains(&needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Synthetic values only.
    const UUID: &str = "00000000-0000-4000-8000-000000000001";
    const AT: &str = "2026-01-01T03:00:00.000Z";
    const RESETS_SECS: i64 = 1_767_243_600; // 2026-01-01T05:00:00Z

    fn hit_line(limit_type: &str) -> String {
        json!({
            "type": "assistant",
            "uuid": UUID,
            "timestamp": AT,
            "isSidechain": false,
            "error": "rate_limit",
            "isApiErrorMessage": true,
            "message": {"content": [{"type": "text", "text": "limit"}]},
            "quotaLimits": {"rateLimitType": limit_type, "resetsAt": RESETS_SECS},
        })
        .to_string()
    }

    fn line(kind: &str) -> String {
        json!({"type": kind, "uuid": "00000000-0000-4000-8000-000000000002",
               "timestamp": AT, "message": {"content": "hi"}})
        .to_string()
    }

    fn text(lines: &[String]) -> Vec<u8> {
        lines.join("\n").into_bytes()
    }

    #[test]
    fn a_limit_error_as_the_last_message_is_a_hit() {
        let bytes = text(&[
            line("user"),
            hit_line("five_hour"),
            json!({"type": "system", "subtype": "turn_duration"}).to_string(),
            json!({"type": "last-prompt"}).to_string(),
            json!({"type": "cost-state"}).to_string(),
        ]);
        let hit = parse_tail(&bytes, false).unwrap();
        assert_eq!(hit.uuid, UUID);
        assert_eq!(hit.at_ms, usage::parse_rfc3339_ms(AT).unwrap());
        assert_eq!(hit.resets_at_ms, RESETS_SECS * 1000);
        assert_eq!(hit.limit().kind, Kind::Session);
        assert_eq!(
            hit.limit().resets_at.as_deref(),
            Some("2026-01-01T05:00:00Z")
        );
        assert_eq!(hit.limit().percent, 100.0);
    }

    #[test]
    fn a_message_after_the_error_is_not_a_hit() {
        let bytes = text(&[hit_line("five_hour"), line("user")]);
        assert_eq!(parse_tail(&bytes, false), None);
        let bytes = text(&[hit_line("five_hour"), line("assistant")]);
        assert_eq!(parse_tail(&bytes, false), None);
    }

    #[test]
    fn a_subagent_line_after_the_error_does_not_hide_it() {
        let mut side: Value = serde_json::from_str(&line("assistant")).unwrap();
        side["isSidechain"] = json!(true);
        let bytes = text(&[hit_line("five_hour"), side.to_string()]);
        assert!(parse_tail(&bytes, false).is_some());
    }

    #[test]
    fn other_errors_and_bad_lines_are_not_hits() {
        let mut other: Value = serde_json::from_str(&hit_line("five_hour")).unwrap();
        other["error"] = json!("overloaded");
        assert_eq!(parse_tail(other.to_string().as_bytes(), false), None);
        let mut no_reset: Value = serde_json::from_str(&hit_line("five_hour")).unwrap();
        no_reset["quotaLimits"] = json!({"rateLimitType": "five_hour"});
        assert_eq!(parse_tail(no_reset.to_string().as_bytes(), false), None);
        assert_eq!(parse_tail(b"{bad json", false), None);
        assert_eq!(parse_tail(b"", false), None);
    }

    #[test]
    fn the_old_error_field_is_the_fallback() {
        let mut value: Value = serde_json::from_str(&hit_line("seven_day")).unwrap();
        let info = value["quotaLimits"].take();
        value.as_object_mut().unwrap().remove("quotaLimits");
        value["apiErrorParams"] = json!({"rate_limit_info": info});
        let hit = parse_tail(value.to_string().as_bytes(), false).unwrap();
        assert_eq!(hit.limit().kind, Kind::WeeklyAll);
    }

    #[test]
    fn a_cut_first_line_is_skipped() {
        let full = hit_line("five_hour");
        let bytes = text(&[full[10..].to_string(), line("user")]);
        assert_eq!(parse_tail(&bytes, true), None);
        let bytes = text(&[line("user")[5..].to_string(), full]);
        assert!(parse_tail(&bytes, true).is_some());
    }

    #[test]
    fn limit_types_map_to_windows() {
        let at = usage::parse_rfc3339_ms(AT).unwrap();
        let hit = |limit_type: &str, hours: i64| Hit {
            uuid: UUID.into(),
            at_ms: at,
            resets_at_ms: at + hours * 3_600_000,
            limit_type: limit_type.into(),
        };
        assert_eq!(hit("five_hour", 2).limit().kind, Kind::Session);
        assert_eq!(hit("seven_day", 50).limit().kind, Kind::WeeklyAll);
        let opus = hit("seven_day_opus", 50).limit();
        assert_eq!(
            (opus.kind, opus.scope.as_deref()),
            (Kind::WeeklyScoped, Some("Opus"))
        );
        // An unknown type uses the time to the reset.
        assert_eq!(hit("new_type", 3).limit().kind, Kind::Session);
        assert_eq!(hit("new_type", 30).limit().kind, Kind::WeeklyAll);
        assert!(hit("five_hour", 2).is_active(at as u64));
        assert!(!hit("five_hour", 2).is_active((at + 2 * 3_600_000) as u64));
    }

    #[test]
    fn read_uses_only_the_end_of_a_large_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let filler = line("user");
        let mut lines: Vec<String> = std::iter::repeat_n(filler, 5000).collect();
        lines.push(hit_line("five_hour"));
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > TAIL);
        assert_eq!(read(&path).unwrap().uuid, UUID);
        assert!(has_line(&path, UUID));
        assert!(!has_line(&path, "00000000-0000-4000-8000-00000000000f"));
        assert_eq!(read(&dir.path().join("missing.jsonl")), None);
    }
}

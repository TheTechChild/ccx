//! `ccx manage spend [--month YYYY-MM] [@name] [--json]` (#24): an estimate
//! of the money that each API-key account used in one month.
//!
//! The report reads only the session transcripts (`projects/**/*.jsonl`) in
//! the dir of each API-key account. It adds the `usage` token counts of the
//! assistant messages in the month, for each model, and multiplies them by
//! a price table. It never reads a key, never runs a command and never
//! makes a network call.

use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Deserialize;
use serde_json::json;

use crate::config::{self, Account, Config, Price, Result};
use crate::env::Env;

/// The note that each report shows.
pub const NOTE: &str = "This is an estimate from local transcripts. ccx does not call a billing API. The totals do not include unpriced models.";

/// The built-in prices in USD for each million tokens: input, output,
/// cache write (5-minute TTL) and cache read. `[pricing.<model>]` in
/// `config.toml` overrides an entry.
const BUILT_IN: &[(&str, [f64; 4])] = &[
    ("claude-fable-5-1", [10.0, 50.0, 12.5, 0.25]),
    ("claude-mythos-5-1", [10.0, 50.0, 12.5, 0.25]),
    ("claude-fable-5", [10.0, 50.0, 12.5, 1.0]),
    ("claude-mythos-5", [10.0, 50.0, 12.5, 1.0]),
    ("claude-opus-5-5", [4.0, 20.0, 5.0, 0.2]),
    ("claude-opus-5", [5.0, 25.0, 6.25, 0.5]),
    ("claude-opus-4-8", [5.0, 25.0, 6.25, 0.5]),
    ("claude-opus-4-7", [5.0, 25.0, 6.25, 0.5]),
    ("claude-opus-4-6", [5.0, 25.0, 6.25, 0.5]),
    ("claude-opus-4-5", [5.0, 25.0, 6.25, 0.5]),
    ("claude-opus-4-1", [15.0, 75.0, 18.75, 1.5]),
    ("claude-opus-4", [15.0, 75.0, 18.75, 1.5]),
    ("claude-sonnet-5-5", [2.0, 10.0, 2.5, 0.2]),
    ("claude-sonnet-5", [2.0, 10.0, 2.5, 0.2]),
    ("claude-sonnet-4-6", [3.0, 15.0, 3.75, 0.3]),
    ("claude-sonnet-4-5", [3.0, 15.0, 3.75, 0.3]),
    ("claude-sonnet-4", [3.0, 15.0, 3.75, 0.3]),
    ("claude-haiku-5-5", [0.1, 0.5, 0.125, 0.01]),
    ("claude-haiku-4-5", [1.0, 5.0, 1.25, 0.1]),
];

/// A calendar month, for example 2026-10.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Month {
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
}

impl Month {
    /// Parses `YYYY-MM`.
    pub fn parse(text: &str) -> std::result::Result<Month, String> {
        let error = || format!("`{text}` is not a month in the format YYYY-MM");
        let (year, month) = text.split_once('-').ok_or_else(error)?;
        if year.len() != 4 || month.len() != 2 {
            return Err(error());
        }
        if !(year.bytes().chain(month.bytes())).all(|b| b.is_ascii_digit()) {
            return Err(error());
        }
        let month: u32 = month.parse().map_err(|_| error())?;
        if !(1..=12).contains(&month) {
            return Err(error());
        }
        Ok(Month {
            year: year.parse().map_err(|_| error())?,
            month,
        })
    }

    /// The month of local time now.
    fn current() -> Result<Month> {
        let tm = crate::pick::local_tm(crate::usage::now_ms() as i64)
            .ok_or("cannot get the local time")?;
        Ok(Month {
            year: tm.tm_year + 1900,
            month: (tm.tm_mon + 1) as u32,
        })
    }

    fn next(self) -> Month {
        if self.month == 12 {
            Month {
                year: self.year + 1,
                month: 1,
            }
        } else {
            Month {
                year: self.year,
                month: self.month + 1,
            }
        }
    }

    /// The start and the end (not included) of the month in local time,
    /// in milliseconds since the Unix epoch.
    fn local_range_ms(self) -> Result<(i64, i64)> {
        let start = local_start_ms(self).ok_or("cannot get the start of the month")?;
        let end = local_start_ms(self.next()).ok_or("cannot get the end of the month")?;
        Ok((start, end))
    }
}

impl std::fmt::Display for Month {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04}-{:02}", self.year, self.month)
    }
}

/// Midnight at the start of day 1 of `month` in local time.
fn local_start_ms(month: Month) -> Option<i64> {
    // SAFETY: `tm` is plain data, so zeroed memory is a valid value.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = month.year - 1900;
    tm.tm_mon = i32::try_from(month.month).ok()? - 1;
    tm.tm_mday = 1;
    tm.tm_isdst = -1;
    // SAFETY: the pointer is valid for the call. `mktime` reads `TZ`.
    let secs = unsafe { libc::mktime(&mut tm) };
    if secs == -1 {
        return None;
    }
    // `time_t` is not `i64` on all targets.
    #[allow(clippy::useless_conversion)]
    let secs = i64::from(secs);
    Some(secs * 1000)
}

/// The token counts of one model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
}

impl Tokens {
    fn add(&mut self, other: Tokens) {
        self.input += other.input;
        self.output += other.output;
        self.cache_write += other.cache_write;
        self.cache_read += other.cache_read;
    }

    fn is_zero(&self) -> bool {
        *self == Tokens::default()
    }

    /// The cost in USD at `price` (USD for each million tokens).
    fn cost(&self, price: &Price) -> f64 {
        (self.input as f64 * price.input
            + self.output as f64 * price.output
            + self.cache_write as f64 * price.cache_write
            + self.cache_read as f64 * price.cache_read)
            / 1_000_000.0
    }
}

/// The token sums of one account.
#[derive(Debug, Default, PartialEq)]
pub struct Scan {
    /// Token sums for each model.
    pub models: BTreeMap<String, Tokens>,
    /// The number of lines that are not valid JSON, or assistant messages
    /// with no valid timestamp, model or usage.
    pub skipped: u64,
}

// The parts of a transcript line that the report uses. serde ignores all
// other fields.

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
}

/// Adds the usage of the assistant messages in `projects/**/*.jsonl` of
/// `dir` with a timestamp in `[start_ms, end_ms)`.
pub fn scan(dir: &Path, start_ms: i64, end_ms: i64) -> Scan {
    let mut files = Vec::new();
    find_jsonl(&dir.join("projects"), &mut files);
    files.sort();
    let mut out = Scan::default();
    // Claude Code writes one line for each content block of a message.
    // Each line has the same message id, request id and usage.
    let mut seen = HashSet::new();
    for file in files {
        let Ok(file) = File::open(&file) else {
            continue;
        };
        let mut reader = BufReader::new(file);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let line = buf.trim_ascii();
            if line.is_empty() {
                continue;
            }
            scan_line(line, start_ms, end_ms, &mut seen, &mut out);
        }
    }
    out
}

fn scan_line(line: &[u8], start_ms: i64, end_ms: i64, seen: &mut HashSet<String>, out: &mut Scan) {
    let parsed: Line = match serde_json::from_slice(line) {
        Ok(parsed) => parsed,
        Err(_) => {
            // Valid JSON of a different shape is not a transcript message
            // that the report uses. Only text that is not JSON is skipped.
            if serde_json::from_slice::<serde::de::IgnoredAny>(line).is_err() {
                out.skipped += 1;
            }
            return;
        }
    };
    if parsed.kind.as_deref() != Some("assistant") {
        return;
    }
    let Some(message) = parsed.message else {
        out.skipped += 1;
        return;
    };
    let at = parsed
        .timestamp
        .as_deref()
        .and_then(crate::usage::parse_rfc3339_ms);
    let (Some(at), Some(model), Some(usage)) = (at, message.model, message.usage) else {
        out.skipped += 1;
        return;
    };
    if at < start_ms || at >= end_ms {
        return;
    }
    let tokens = Tokens {
        input: usage.input_tokens.unwrap_or(0),
        output: usage.output_tokens.unwrap_or(0),
        cache_write: usage.cache_creation_input_tokens.unwrap_or(0),
        cache_read: usage.cache_read_input_tokens.unwrap_or(0),
    };
    // A message with no tokens, for example a `<synthetic>` error, costs
    // nothing.
    if tokens.is_zero() {
        return;
    }
    if let Some(id) = message.id {
        let key = format!("{id}\n{}", parsed.request_id.unwrap_or_default());
        if !seen.insert(key) {
            return;
        }
    }
    out.models.entry(model).or_default().add(tokens);
}

/// Adds each `*.jsonl` file under `dir` to `files`. It does not follow
/// symlinks.
fn find_jsonl(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            find_jsonl(&path, files);
        } else if kind.is_file() && path.extension().is_some_and(|e| e == "jsonl") {
            files.push(path);
        }
    }
}

/// The model name without a date suffix, for example
/// `claude-opus-4-5-20251101` → `claude-opus-4-5`.
fn without_date(model: &str) -> &str {
    match model.rsplit_once('-') {
        Some((stem, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => stem,
        _ => model,
    }
}

/// The price of `model`: first `[pricing]` with the full name, then
/// `[pricing]` with the name with no date suffix, then the built-in table.
pub fn price(model: &str, overrides: &BTreeMap<String, Price>) -> Option<Price> {
    let short = without_date(model);
    if let Some(price) = overrides.get(model).or_else(|| overrides.get(short)) {
        return Some(*price);
    }
    BUILT_IN.iter().find(|(name, _)| *name == short).map(
        |(_, [input, output, cache_write, cache_read])| Price {
            input: *input,
            output: *output,
            cache_write: *cache_write,
            cache_read: *cache_read,
        },
    )
}

/// One row of the report: the tokens and the cost of one model.
#[derive(Debug, PartialEq)]
pub struct Row {
    pub model: String,
    pub tokens: Tokens,
    /// `None` for a model with no price.
    pub cost: Option<f64>,
}

#[derive(Debug, PartialEq)]
pub struct AccountReport {
    pub name: String,
    pub rows: Vec<Row>,
    pub skipped: u64,
}

impl AccountReport {
    fn total(&self) -> f64 {
        self.rows
            .iter()
            .filter_map(|r| r.cost)
            .fold(0.0, |sum, cost| sum + cost)
    }
}

#[derive(Debug, PartialEq)]
pub struct Report {
    pub month: Month,
    pub accounts: Vec<AccountReport>,
}

impl Report {
    fn total(&self) -> f64 {
        self.accounts
            .iter()
            .map(AccountReport::total)
            .fold(0.0, |sum, cost| sum + cost)
    }

    fn skipped(&self) -> u64 {
        self.accounts.iter().map(|a| a.skipped).sum()
    }
}

/// Makes the rows of one account from its scan.
pub fn account_report(
    name: &str,
    scan: Scan,
    overrides: &BTreeMap<String, Price>,
) -> AccountReport {
    let rows = scan
        .models
        .into_iter()
        .map(|(model, tokens)| {
            let cost = price(&model, overrides).map(|p| tokens.cost(&p));
            Row {
                model,
                tokens,
                cost,
            }
        })
        .collect();
    AccountReport {
        name: name.to_string(),
        rows,
        skipped: scan.skipped,
    }
}

/// The API-key accounts of the report: all of them, or the one in
/// `only` (with or without `@`).
fn selected<'a>(config: &'a Config, only: Option<&str>) -> Result<Vec<&'a Account>> {
    let Some(name) = only else {
        return Ok(config.accounts.iter().filter(|a| a.is_paid()).collect());
    };
    let name = name.strip_prefix('@').unwrap_or(name);
    let account = config.account(name).ok_or_else(|| {
        format!(
            "unknown account `{name}`; known accounts: {}",
            config.names().join(", ")
        )
    })?;
    if !account.is_paid() {
        return Err(format!(
            "`{name}` is not an API-key account; the spend report shows only accounts with kind = \"api_key\""
        ));
    }
    Ok(vec![account])
}

/// Makes the report for `month` from the transcripts on disk.
pub fn build(config: &Config, month: Month, only: Option<&str>) -> Result<Report> {
    let (start, end) = month.local_range_ms()?;
    let accounts = selected(config, only)?
        .into_iter()
        .map(|account| {
            account_report(
                &account.name,
                scan(&account.dir, start, end),
                &config.pricing,
            )
        })
        .collect();
    Ok(Report { month, accounts })
}

pub fn run(env: &Env, month: Option<&str>, only: Option<&str>, as_json: bool) -> Result<()> {
    let config = config::load(env)?;
    let month = match month {
        Some(text) => Month::parse(text)?,
        None => Month::current()?,
    };
    let report = build(&config, month, only)?;
    if as_json {
        println!("{}", to_json(&report));
    } else {
        print!("{}", to_text(&report));
    }
    Ok(())
}

/// USD rounded to 6 decimals, so that the JSON has no float noise.
fn usd(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

/// Stable JSON. The keys of each object are in alphabetical order.
pub fn to_json(report: &Report) -> String {
    let accounts: Vec<_> = report
        .accounts
        .iter()
        .map(|account| {
            let models: Vec<_> = account
                .rows
                .iter()
                .map(|row| {
                    json!({
                        "model": row.model,
                        "priced": row.cost.is_some(),
                        "cost_usd": row.cost.map(usd),
                        "input_tokens": row.tokens.input,
                        "output_tokens": row.tokens.output,
                        "cache_write_tokens": row.tokens.cache_write,
                        "cache_read_tokens": row.tokens.cache_read,
                    })
                })
                .collect();
            json!({
                "name": account.name,
                "models": models,
                "total_usd": usd(account.total()),
                "skipped_lines": account.skipped,
            })
        })
        .collect();
    let value = json!({
        "month": report.month.to_string(),
        "currency": "USD",
        "estimate": true,
        "note": NOTE,
        "accounts": accounts,
        "total_usd": usd(report.total()),
        "skipped_lines": report.skipped(),
    });
    serde_json::to_string_pretty(&value).expect("JSON values always serialize")
}

/// A table with one row for each account and model, a total row for each
/// account and a grand total. The note is at the end.
pub fn to_text(report: &Report) -> String {
    let mut out = format!(
        "Spend of API-key accounts in {} (estimate from local transcripts)\n\n",
        report.month
    );
    if report.accounts.is_empty() {
        out.push_str("No account has kind = \"api_key\".\n\n");
        out.push_str(NOTE);
        out.push('\n');
        return out;
    }
    let header = [
        "ACCOUNT",
        "MODEL",
        "INPUT",
        "OUTPUT",
        "CACHE WRITE",
        "CACHE READ",
        "COST",
    ];
    let mut rows: Vec<[String; 7]> = vec![header.map(String::from)];
    for account in &report.accounts {
        for row in &account.rows {
            let t = row.tokens;
            rows.push([
                account.name.clone(),
                row.model.clone(),
                t.input.to_string(),
                t.output.to_string(),
                t.cache_write.to_string(),
                t.cache_read.to_string(),
                row.cost.map_or_else(|| "unpriced".into(), dollars),
            ]);
        }
        let mut total = [(); 7].map(|_| String::new());
        total[0] = account.name.clone();
        total[1] = "total".into();
        total[6] = dollars(account.total());
        rows.push(total);
    }
    let mut total = [(); 7].map(|_| String::new());
    total[0] = "TOTAL".into();
    total[6] = dollars(report.total());
    rows.push(total);

    let mut widths = [0; 7];
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    for row in &rows {
        let cells: Vec<String> = row
            .iter()
            .zip(widths)
            .enumerate()
            .map(|(i, (cell, width))| {
                if i < 2 {
                    format!("{cell:<width$}")
                } else {
                    format!("{cell:>width$}")
                }
            })
            .collect();
        out.push_str(cells.join("  ").trim_end());
        out.push('\n');
    }
    out.push('\n');
    let skipped = report.skipped();
    if skipped > 0 {
        let lines = if skipped == 1 { "line" } else { "lines" };
        out.push_str(&format!(
            "Skipped {skipped} malformed or partial transcript {lines}.\n"
        ));
    }
    out.push_str(NOTE);
    out.push('\n');
    out
}

/// `$12.34`.
fn dollars(value: f64) -> String {
    format!("${value:.2}")
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-10-01T00:00:00Z and 2026-11-01T00:00:00Z.
    const OCT: (i64, i64) = (1_790_812_800_000, 1_793_491_200_000);

    fn assistant(ts: &str, id: &str, model: &str, usage: [u64; 4]) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","requestId":"req_{id}","message":{{"id":"msg_{id}","model":"{model}","usage":{{"input_tokens":{},"output_tokens":{},"cache_creation_input_tokens":{},"cache_read_input_tokens":{}}}}}}}"#,
            usage[0], usage[1], usage[2], usage[3]
        )
    }

    fn write(dir: &Path, rel: &str, lines: &[String]) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, lines.join("\n") + "\n").unwrap();
    }

    #[test]
    fn the_test_month_bounds_are_correct() {
        let start = crate::usage::parse_rfc3339_ms("2026-10-01T00:00:00Z").unwrap();
        let end = crate::usage::parse_rfc3339_ms("2026-11-01T00:00:00Z").unwrap();
        assert_eq!((start, end), OCT);
    }

    #[test]
    fn month_parses_only_yyyy_mm() {
        assert_eq!(
            Month::parse("2026-10"),
            Ok(Month {
                year: 2026,
                month: 10
            })
        );
        for bad in [
            "2026-13", "2026-00", "2026-1", "26-10", "2026/10", "2026-1a", "",
        ] {
            assert!(Month::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(
            Month::parse("2026-12").unwrap().next().to_string(),
            "2027-01"
        );
    }

    #[test]
    fn scan_sums_tokens_per_model_in_the_month_only() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "projects/-home-a/s1.jsonl",
            &[
                r#"{"type":"user","timestamp":"2026-10-02T00:00:00Z","message":{"role":"user","content":"hi"}}"#.into(),
                assistant("2026-10-02T10:00:00.000Z", "a", "claude-opus-5-5", [10, 20, 30, 40]),
                // A second content block of the same message: not counted again.
                assistant("2026-10-02T10:00:01.000Z", "a", "claude-opus-5-5", [10, 20, 30, 40]),
                assistant("2026-10-31T23:59:59.999Z", "b", "claude-opus-5-5", [1, 2, 3, 4]),
                // Out of the month.
                assistant("2026-09-30T23:59:59.999Z", "c", "claude-opus-5-5", [100, 0, 0, 0]),
                assistant("2026-11-01T00:00:00.000Z", "d", "claude-opus-5-5", [100, 0, 0, 0]),
            ],
        );
        write(
            dir.path(),
            "projects/-home-b/s2/subagents/agent-1.jsonl",
            &[assistant(
                "2026-10-05T00:00:00Z",
                "e",
                "claude-haiku-4-5-20251001",
                [5, 6, 7, 8],
            )],
        );
        let scan = scan(dir.path(), OCT.0, OCT.1);
        assert_eq!(scan.skipped, 0);
        assert_eq!(
            scan.models["claude-opus-5-5"],
            Tokens {
                input: 11,
                output: 22,
                cache_write: 33,
                cache_read: 44
            }
        );
        assert_eq!(
            scan.models["claude-haiku-4-5-20251001"],
            Tokens {
                input: 5,
                output: 6,
                cache_write: 7,
                cache_read: 8
            }
        );
        assert_eq!(scan.models.len(), 2);
    }

    #[test]
    fn scan_skips_and_counts_malformed_and_partial_lines() {
        let dir = tempfile::tempdir().unwrap();
        let good = assistant("2026-10-02T10:00:00Z", "a", "claude-opus-5-5", [1, 1, 1, 1]);
        let partial = good[..good.len() / 2].to_string();
        write(
            dir.path(),
            "projects/p/s.jsonl",
            &[
                "not json".into(),
                good,
                // Assistant messages with no timestamp and with no usage.
                r#"{"type":"assistant","message":{"model":"m","usage":{"input_tokens":1}}}"#.into(),
                r#"{"type":"assistant","timestamp":"2026-10-02T10:00:00Z","message":{"model":"m"}}"#.into(),
                // Valid JSON of another shape is not malformed.
                "[1,2,3]".into(),
                "".into(),
                partial,
            ],
        );
        let scan = scan(dir.path(), OCT.0, OCT.1);
        assert_eq!(scan.skipped, 4);
        assert_eq!(scan.models["claude-opus-5-5"].input, 1);
    }

    #[test]
    fn scan_ignores_zero_usage_and_a_missing_projects_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(scan(dir.path(), OCT.0, OCT.1), Scan::default());
        write(
            dir.path(),
            "projects/p/s.jsonl",
            &[assistant(
                "2026-10-02T10:00:00Z",
                "a",
                "<synthetic>",
                [0, 0, 0, 0],
            )],
        );
        assert_eq!(scan(dir.path(), OCT.0, OCT.1), Scan::default());
    }

    #[test]
    fn price_uses_the_built_in_table_with_or_without_a_date() {
        let none = BTreeMap::new();
        assert_eq!(price("claude-opus-5-5", &none).unwrap().input, 4.0);
        assert_eq!(price("claude-opus-4-5-20251101", &none).unwrap().input, 5.0);
        // `claude-opus-4` is not a prefix match for a different model.
        assert_eq!(price("claude-opus-4-20250514", &none).unwrap().input, 15.0);
        assert_eq!(price("claude-opus-4-9", &none), None);
        assert_eq!(price("mystery", &none), None);
    }

    #[test]
    fn config_pricing_overrides_the_built_in_table() {
        let one = Price {
            input: 1.0,
            output: 1.0,
            cache_write: 1.0,
            cache_read: 1.0,
        };
        let two = Price { input: 2.0, ..one };
        let overrides = BTreeMap::from([
            ("claude-opus-5-5".to_string(), one),
            ("claude-sonnet-4-5-20250929".to_string(), two),
            ("mystery".to_string(), two),
        ]);
        assert_eq!(price("claude-opus-5-5", &overrides), Some(one));
        assert_eq!(price("claude-opus-5-5-20260101", &overrides), Some(one));
        assert_eq!(price("claude-sonnet-4-5-20250929", &overrides), Some(two));
        assert_eq!(price("mystery", &overrides), Some(two));
        // Other models keep the built-in price.
        assert_eq!(price("claude-haiku-4-5", &overrides).unwrap().input, 1.0);
        assert_eq!(price("claude-sonnet-4-5", &overrides).unwrap().input, 3.0);
    }

    #[test]
    fn costs_use_the_price_and_unpriced_models_stay_out_of_the_totals() {
        let scan = Scan {
            models: BTreeMap::from([
                (
                    "claude-opus-5-5".to_string(),
                    Tokens {
                        input: 1_000_000,
                        output: 100_000,
                        cache_write: 200_000,
                        cache_read: 5_000_000,
                    },
                ),
                (
                    "mystery".to_string(),
                    Tokens {
                        input: 7,
                        output: 0,
                        cache_write: 0,
                        cache_read: 0,
                    },
                ),
            ]),
            skipped: 0,
        };
        let report = account_report("paid", scan, &BTreeMap::new());
        // 4 + 0.1 * 20 + 0.2 * 5 + 5 * 0.2 = 8.
        assert!((report.rows[0].cost.unwrap() - 8.0).abs() < 1e-9);
        assert_eq!(report.rows[1].cost, None);
        assert_eq!(report.rows[1].tokens.input, 7);
        assert!((report.total() - 8.0).abs() < 1e-9);
    }
}

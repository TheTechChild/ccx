//! Automatic handoff on `ccx -c`, `ccx --continue` and `ccx --resume <id>`
//! (#15).
//!
//! ccx finds the account that owns the session from the files on disk. When
//! the owner is not limited, ccx launches the owner. When the owner is
//! limited, or is a paid API-key account (#20), ccx picks a target with
//! automatic selection, copies the session with `ccx handoff` (#14) and
//! resumes it there. When all subscription accounts are limited, ccx can
//! offer a paid account; else it launches the owner with a warning. A bare
//! `--resume` opens the session picker of Claude Code, so it never does a
//! handoff.

use std::ffi::OsString;

use crate::config::{self, Config};
use crate::handoff::{self, Thread};
use crate::pick::{self, Candidate};

/// The resume request in the claude arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub kind: Kind,
    /// The index of the first argument of the request.
    at: usize,
    /// The number of arguments of the request: 1 or 2.
    len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// `-c` or `--continue`: the newest session for the cwd.
    Continue,
    /// `-r <id>`, `--resume <id>` or `--resume=<id>`.
    Resume(String),
}

/// True for a full session id: a UUID with 36 characters.
fn is_full_id(text: &str) -> bool {
    text.len() == 36 && handoff::is_session_id(text)
}

/// Finds the first resume request before `--`. A bare `--resume`, or a
/// `--resume` with a value that is not a full session id (a search term),
/// gives `None`: Claude Code then shows its picker, and ccx cannot know the
/// session.
pub fn intent(args: &[OsString]) -> Option<Intent> {
    for (at, arg) in args.iter().enumerate() {
        let Some(arg) = arg.to_str() else {
            continue;
        };
        match arg {
            "--" => return None,
            "-c" | "--continue" => {
                return Some(Intent {
                    kind: Kind::Continue,
                    at,
                    len: 1,
                });
            }
            "-r" | "--resume" => {
                let id = args.get(at + 1).and_then(|v| v.to_str());
                return id.filter(|id| is_full_id(id)).map(|id| Intent {
                    kind: Kind::Resume(id.to_string()),
                    at,
                    len: 2,
                });
            }
            _ => {
                if let Some(id) = arg.strip_prefix("--resume=") {
                    return is_full_id(id).then(|| Intent {
                        kind: Kind::Resume(id.to_string()),
                        at,
                        len: 1,
                    });
                }
            }
        }
    }
    None
}

/// The session that the request names and the account that owns it.
/// `enc` is the encoded cwd, for `-c`. When more than one account has the
/// session, the newest copy wins. `None` when no account has it.
pub fn owner(config: &Config, kind: &Kind, enc: Option<&str>) -> Option<Thread> {
    match kind {
        Kind::Continue => handoff::list(config, Some(enc?)).into_iter().next(),
        Kind::Resume(id) => handoff::find_by_id(config, id),
    }
}

/// The claude arguments for the target account: the request becomes
/// `--resume <id>`, so that claude opens the copy (or the fork) and not a
/// different session of the target.
pub fn target_args(args: &[OsString], intent: &Intent, id: &str) -> Vec<OsString> {
    let mut out = args.to_vec();
    let end = (intent.at + intent.len).min(out.len());
    out.splice(
        intent.at..end,
        [OsString::from("--resume"), OsString::from(id)],
    );
    out
}

/// Where the session goes.
#[derive(Debug, PartialEq)]
pub enum Route {
    /// The owner is a subscription account that is not limited: launch the
    /// owner and copy nothing.
    Owner,
    /// No subscription account other than the owner is usable. `decision`
    /// is the pick with the soonest reset, or `None` when there is no
    /// subscription candidate. ccx can offer a paid account (#20).
    AllLimited { decision: Option<pick::Decision> },
    /// Hand off to `candidates[index]`. `reason` tells why the session
    /// leaves the owner, for example `main limited: 5h 92%`.
    Handoff { index: usize, reason: String },
}

/// Selects the route with the rules of automatic selection (#10, #20).
/// `candidates` are the enabled accounts, the same as for `ccx` with no
/// arguments. A paid owner is never a first choice: the session goes to a
/// usable subscription account.
pub fn route(
    candidates: &[Candidate],
    owner: &Candidate,
    model_arg: Option<&str>,
    config: &config::Pick,
    now_ms: u64,
) -> Route {
    let model = model_arg.or(owner.settings_model.as_deref());
    let reason = if owner.paid {
        paid_reason(&owner.name)
    } else {
        let Some(usage) = owner
            .usage
            .as_ref()
            .filter(|u| pick::limited(Some(u), config, model, now_ms))
        else {
            return Route::Owner;
        };
        format!(
            "{} limited: {}",
            owner.name,
            pick::limit_text(usage, config, model, now_ms)
        )
    };
    match pick::pick(candidates, model_arg, config, now_ms) {
        Some(decision)
            if !matches!(decision.reason, pick::Reason::AllLimited { .. })
                && candidates[decision.index].name != owner.name =>
        {
            Route::Handoff {
                index: decision.index,
                reason,
            }
        }
        Some(decision) if matches!(decision.reason, pick::Reason::AllLimited { .. }) => {
            Route::AllLimited {
                decision: Some(decision),
            }
        }
        _ => Route::AllLimited { decision: None },
    }
}

/// The handoff reason when the owner is a paid API-key account.
pub fn paid_reason(owner: &str) -> String {
    format!("{owner} is a paid API-key account")
}

/// The handoff banner, for example
/// `ccx: handoff main → angel2 (main limited: 5h 92%)`. A fork adds the new
/// session id.
pub fn banner(owner: &str, target: &str, reason: &str, fork: Option<&str>) -> String {
    let mut text = format!("ccx: handoff {owner} → {target} ({reason})");
    if let Some(id) = fork {
        text.push_str(&format!(
            "; {target} had a different copy, new session id {id}"
        ));
    }
    text
}

/// The warning when all accounts are limited.
pub fn all_limited_warning(owner: &str) -> String {
    format!("ccx: warning: all accounts limited; the session stays in {owner}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{Limit, Snapshot, Source};

    const ID: &str = "11111111-1111-4111-8111-111111111111";
    const NOW: u64 = 1_767_225_600_000;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn cfg() -> config::Pick {
        config::Pick {
            session_avoid: 90,
            weekly_avoid: 95,
            scoped_avoid: 98,
        }
    }

    fn cand(name: &str, session: Option<f64>) -> Candidate {
        Candidate {
            name: name.into(),
            usage: session.map(|percent| Snapshot {
                source: Source::Usage,
                fetched_at_ms: NOW,
                limits: vec![Limit {
                    kind: crate::usage::Kind::Session,
                    percent,
                    resets_at: Some("2099-01-01T00:00:00Z".into()),
                    scope: None,
                }],
            }),
            settings_model: None,
            paid: false,
            running: 0,
        }
    }

    #[test]
    fn intent_finds_continue_and_resume_with_an_id() {
        let resume = |at, len| {
            Some(Intent {
                kind: Kind::Resume(ID.into()),
                at,
                len,
            })
        };
        let cont = |at| {
            Some(Intent {
                kind: Kind::Continue,
                at,
                len: 1,
            })
        };
        assert_eq!(intent(&os(&["-c"])), cont(0));
        assert_eq!(intent(&os(&["--model", "opus", "--continue"])), cont(2));
        assert_eq!(intent(&os(&["--resume", ID])), resume(0, 2));
        assert_eq!(intent(&os(&["-p", "-r", ID, "hi"])), resume(1, 2));
        assert_eq!(intent(&os(&[&format!("--resume={ID}")])), resume(0, 1));
    }

    #[test]
    fn intent_is_none_for_a_bare_resume_and_other_args() {
        for args in [
            &[][..],
            &["--resume"],
            &["--resume", "--model", "opus"],
            &["--resume", "search words"],
            &["--resume", &ID[..8]],
            &["--resume="],
            &["--", "-c"],
            &["--model", "opus"],
            &["-cx"],
        ] {
            assert_eq!(intent(&os(args)), None, "{args:?}");
        }
    }

    #[test]
    fn target_args_use_the_new_id() {
        let new = "22222222-2222-4222-8222-222222222222";
        let cases: [(&[&str], &[&str]); 4] = [
            (&["-c"], &["--resume", new]),
            (
                &["--model", "opus", "-c", "hi"],
                &["--model", "opus", "--resume", new, "hi"],
            ),
            (&["--resume", ID, "-p"], &["--resume", new, "-p"]),
            (&[&format!("--resume={ID}")], &["--resume", new]),
        ];
        for (args, want) in cases {
            let args = os(args);
            let intent = intent(&args).unwrap();
            assert_eq!(target_args(&args, &intent, new), os(want), "{args:?}");
        }
    }

    #[test]
    fn route_rules() {
        let route =
            |cands: &[Candidate], owner: usize| route(cands, &cands[owner], None, &cfg(), NOW);
        // The owner is not limited, also when a different account is better.
        assert_eq!(
            route(&[cand("a", Some(80.0)), cand("b", Some(0.0))], 0),
            Route::Owner
        );
        // Unknown usage is never limited.
        assert_eq!(
            route(&[cand("a", None), cand("b", Some(0.0))], 0),
            Route::Owner
        );
        // The owner is limited: the best other account.
        assert_eq!(
            route(&[cand("a", Some(92.0)), cand("b", Some(10.0))], 0),
            Route::Handoff {
                index: 1,
                reason: "a limited: 5h 92%".into()
            }
        );
        // All accounts are limited: the soonest reset is in the route.
        assert!(matches!(
            route(&[cand("a", Some(92.0)), cand("b", Some(99.0))], 0),
            Route::AllLimited {
                decision: Some(pick::Decision { index: 0, .. })
            }
        ));
        // A limited owner that is not a candidate (a disabled account).
        let owner = cand("off", Some(95.0));
        assert_eq!(
            super::route(&[cand("b", Some(0.0))], &owner, None, &cfg(), NOW),
            Route::Handoff {
                index: 0,
                reason: "off limited: 5h 95%".into()
            }
        );
        assert_eq!(
            super::route(&[], &owner, None, &cfg(), NOW),
            Route::AllLimited { decision: None }
        );
    }

    fn paid(name: &str) -> Candidate {
        Candidate {
            paid: true,
            ..cand(name, None)
        }
    }

    #[test]
    fn a_paid_owner_goes_to_a_usable_subscription_account() {
        // The first subscription account that is not limited, in the fill
        // order. The API-key account is last.
        let cands = [
            cand("a", Some(92.0)),
            cand("b", Some(80.0)),
            cand("c", Some(10.0)),
            paid("paid"),
        ];
        assert_eq!(
            route(&cands, &cands[3], None, &cfg(), NOW),
            Route::Handoff {
                index: 1,
                reason: "paid is a paid API-key account".into()
            }
        );
    }

    #[test]
    fn a_paid_owner_with_all_subscription_accounts_limited() {
        let cands = [paid("paid"), cand("a", Some(95.0))];
        assert!(matches!(
            route(&cands, &cands[0], None, &cfg(), NOW),
            Route::AllLimited {
                decision: Some(pick::Decision { index: 1, .. })
            }
        ));
        // Only paid accounts: no subscription account to go to.
        let cands = [paid("paid")];
        assert_eq!(
            route(&cands, &cands[0], None, &cfg(), NOW),
            Route::AllLimited { decision: None }
        );
    }

    #[test]
    fn banner_and_warning_text() {
        assert_eq!(
            banner("main", "angel2", "main limited: 5h 92%", None),
            "ccx: handoff main → angel2 (main limited: 5h 92%)"
        );
        assert_eq!(
            banner("main", "angel2", "main limited: wk 96%", Some(ID)),
            format!(
                "ccx: handoff main → angel2 (main limited: wk 96%); angel2 had a different copy, new session id {ID}"
            )
        );
        assert_eq!(
            all_limited_warning("main"),
            "ccx: warning: all accounts limited; the session stays in main"
        );
    }
}

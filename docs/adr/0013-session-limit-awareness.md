# 0013. Running sessions and limit hits change the pick

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

The session window (5 hours) belongs to the account. All sessions on one account use the same window. On 2026-10-23, three sessions ran on one Max 5x account at the same time. They filled the session window together, and all three stopped within two minutes. The weekly usage of the account was only 14%.

The score of [automatic selection](../launch.md#automatic-selection) already uses the higher of the session % and the weekly %. But three gaps let this occur:

1. `ccx @name` starts the account with no check. ccx showed no warning for an account at 100% session usage.
2. The score uses only the usage data. It does not know how many sessions already use the account. A second and a third launch can select the same account, and the sessions then use the window faster.
3. The usage data can be up to `usage.fresh_minutes` old (default 15). When the launch fetch does not finish in `usage.fetch_timeout_secs`, ccx uses older data. A running session cannot move to a different account. Thus ccx can resume a session on an account that is already full.

## Decision

1. **Named launch warning.** On a terminal, `ccx @name` shows a warning when the account is limited, for example `ccx: warning: angel2 is limited (5h 100%); it frees 17:29 (56m). Run ccx with no @name to pick an account.` ccx still starts the named account. The name is the choice of the user, and a question would stop scripts. The check reads only local files.
2. **Running sessions share the score.** The score is (100 − the higher of session % and weekly %) × weight ÷ (running sessions + 1). A running session is a `sessions/<pid>.json` file ([ADR 0006](0006-m0-session-file-layout.md)) with a live `pid`. This count uses only the pid check (`kill(pid, 0)`), not `ps` for each file, so that the launch stays fast. A pid that the system used again can count as a running session. This error makes the score of the account a little lower only. ccx never reads `sessions/*.key`. The banner shows the count, for example `1 running`.
3. **A limit hit in the transcript limits the account.** When Claude Code stops a turn at a usage limit, it writes an assistant line with `"error": "rate_limit"` and `quotaLimits` (`rateLimitType`, `resetsAt` in seconds). ccx reads the last 256 KiB of one transcript: the session that `-c` or `--resume <id>` opens, or else the newest session in the cwd. When the last message is a limit hit, the reset is in the future, and the hit is newer than the usage data, ccx sets that window of the account to 100% until the reset. `five_hour` is the session window, `seven_day` is the weekly window, and `seven_day_opus` and `seven_day_sonnet` are scoped windows.
   - A handoff copies the transcript with its hit line. When a transcript of a different account has the same line `uuid`, ccx cannot know which account hit the limit, so it ignores the hit.
   - After `ccx` with no arguments picks a different account, ccx shows a note: `ccx: the last session here stopped at the angel2 usage limit. To continue it on a different account, run ccx -c.`

## Consequences

- `ccx -c` and `ccx --resume <id>` hand off a session that stopped at a limit, also when the usage cache is old.
- New sessions spread over the accounts by their capacity. An account with two running sessions scores one third of its free capacity.
- The launch reads more files: the `sessions/` directory of each account, one directory of transcripts for each account, and the end of one or two transcripts. These are local reads with no network, so the launch stays fast.
- ccx depends on more Claude Code file formats: the `sessions/<pid>.json` `pid` field and the fields of the limit-hit line. If Claude Code changes them, ccx falls back to the old behavior: no count and no hit.
- ccx cannot tell a running session about a limit, because ccx replaces itself with `claude` (`exec`). The note and `ccx -c` are the path to continue.

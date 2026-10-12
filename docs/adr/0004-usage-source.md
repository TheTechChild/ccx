# 0004. Usage source is `/usage` through `claude -p`

- **Status:** Accepted
- **Date:** 2026-10-12

## Context

ccx selects the account that has the most usage available. Thus, ccx needs the current usage for each account. ccx must not use credentials (refer to [ADR 0002](0002-no-credential-access.md)).

Claude Code shows the usage limits with the `/usage` command. In print mode with `stream-json` output, the assistant event contains `usage_report.rate_limits.limits[]`. Each limit has a kind: `session`, `weekly_all` or `weekly_scoped`.

Claude Code also keeps a `cachedUsageUtilization` value in the `.claude.json` file of each account. This value can be many hours old.

## Decision

- The primary usage source is this command, with the configuration directory of the account:

  ```sh
  claude -p "/usage" --output-format stream-json --verbose --no-session-persistence --strict-mcp-config
  ```

- ccx runs the command from a fixed neutral directory and with hooks off.
- If the command fails, ccx uses `cachedUsageUtilization` from `.claude.json` and shows the age of the value.
- ccx keeps the results in a cache. A direct call to the usage API is a possible follow-up only.

## Consequences

- ccx needs no credentials to get usage data.
- A usage fetch starts a Claude Code process, so it is slow. ccx must use the cache and refresh it in the background, so that the launch stays fast.
- If Claude Code changes the `stream-json` format, the fetch fails. The fallback continues to give older data.

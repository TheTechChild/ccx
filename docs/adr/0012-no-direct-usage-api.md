# 0012. ccx does not call the usage API directly

- **Status:** Accepted
- **Date:** 2026-10-12
- **Issue:** #23
- **Decision:** D6

## Context

[ADR 0004](0004-usage-source.md) selects `claude -p "/usage"` as the usage source. That command starts a Claude Code process, so it is slow. ADR 0004 names a direct call to `GET https://api.anthropic.com/api/oauth/usage` as a possible follow-up. Issue #23 is the policy review of that follow-up.

## Decision

ccx does not call `/api/oauth/usage` (decision D6: no-go). ccx has no code for this source.

## Reasons

- The endpoint has no documentation. Anthropic can change it or remove it with no notice.
- The endpoint needs the Claude Code OAuth token. To get the token, ccx must read it from the macOS keychain or from `.credentials.json`. ccx never reads, keeps or refreshes Claude Code credentials ([ADR 0002](0002-no-credential-access.md)). This source breaks that basic rule.
- The Claude Code legal page says that developers must not collect, keep or intermediate Claude.ai credentials. Section 3 of the Consumer Terms limits automated access.
- The current source is sufficient. `claude -p "/usage" --output-format stream-json`, with the `cachedUsageUtilization` fallback from `.claude.json` (#9), gives the usage data with no credentials. The cache and the background refresh keep the launch fast.

## Consequences

- The guard test in `tests/no_credentials.rs` stays as it is. It fails when non-test code contains `oauth/token`. It has no exception for token reads.
- A usage fetch continues to start a Claude Code process.
- If Anthropic documents a usage API that does not need ccx to touch the token, a new ADR can examine it.

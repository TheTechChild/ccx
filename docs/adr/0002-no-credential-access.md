# 0002. ccx does not touch Claude Code credentials

- **Status:** Accepted
- **Date:** 2026-10-12

## Context

Claude Code keeps OAuth credentials in the macOS keychain, or in `<config dir>/.credentials.json` on Linux. A tool that reads, copies or refreshes these tokens can cause a security risk. It can also break a login that is in use on a different machine, because a refresh can make the old token invalid.

## Decision

- ccx never reads, keeps, copies or refreshes Claude Code credentials.
- All logins and logouts go through `claude auth login` and `claude auth logout`, with the configuration directory of the account.
- ccx gets account data only from Claude Code commands (for example, `claude auth status`) and from non-secret files.

## Consequences

- A security error in ccx cannot expose a token.
- ccx cannot use the usage API directly with a token. It must get usage data through Claude Code (refer to [ADR 0004](0004-usage-source.md)).
- Tests must not read keychain entries or `.credentials.json` files.

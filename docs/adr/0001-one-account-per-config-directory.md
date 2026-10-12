# 0001. One account for each configuration directory

- **Status:** Accepted
- **Date:** 2026-10-12

## Context

Claude Code keeps all data for one login in one configuration directory. The `CLAUDE_CONFIG_DIR` environment variable selects the directory. The Claude Code documentation gives this method for more than one account.

When `CLAUDE_CONFIG_DIR` is not set, Claude Code uses `~/.claude` and keeps the identity file at `~/.claude.json`. When the variable is set, Claude Code keeps the identity file in that directory. On macOS, the keychain entry name comes from a hash of the exact path. Thus, if you set `CLAUDE_CONFIG_DIR=~/.claude`, Claude Code uses a different identity file and a different keychain entry. This split occurred one time before, and it left a stray `~/.claude/.claude.json`.

## Decision

- Each account is one configuration directory. ccx keeps a map from account name to directory in its configuration file.
- For the base account (`~/.claude`), ccx removes `CLAUDE_CONFIG_DIR` from the environment. It does not set the variable to `~/.claude`.
- For all other accounts, ccx sets `CLAUDE_CONFIG_DIR` to the absolute path. The path has no trailing slash, and ccx does not resolve symbolic links in it.
- The bare `claude` command continues to use the base account.

## Consequences

- ccx uses the documented Claude Code method. It needs no special login logic.
- ccx must keep the path in canonical form. A different form of the same path is a different account for Claude Code.
- ccx must remove environment variables that override the selected directory or the selected login.

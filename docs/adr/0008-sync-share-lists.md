# 0008. Sync share lists and the shared `plugins/synced` risk

- **Status:** Accepted
- **Date:** 2026-10-12
- **Issue:** #7

## Context

`ccx manage sync` links the setup of the base directory `~/.claude` into the other accounts (refer to [ADR 0003](0003-link-all-sharing.md)). Sync must know which items to link and which items to keep in each account. Each top-level entry of the base directory goes into one of three groups: shared, never shared or unknown.

M0 test 2 passed (refer to [ADR 0007](0007-m0-manual-tests.md)). A `/config` write keeps the `settings.json` link. Thus, the D4 fallback for test 2 is not necessary. A `settings.json` that is a real file is a normal CONFLICT item.

## Decision

### Shared: link as one item

Sync links each of these items as one symlink with an absolute target in the base directory:

- `settings.json`
- `CLAUDE.md`
- `keybindings.json`
- `statusline.py`
- `agents/`
- `commands/`
- `output-styles/`
- `plugins/`

### Shared: one link for each skill

`skills/` is a real directory in each account. Sync makes one symlink for each child of the base `skills/`. Sync never links `skills/synced`, because Claude Code keys it per organization and account. Sync also ignores the dotfiles in `skills/`, for example `.DS_Store`. An account can have local skills next to the links.

### Never shared

Sync never links these items:

- `.claude.json`, `.claude.json.lock`, `.credentials.json` and all other dotfiles
- `projects/`, `history.jsonl`, `file-history/`, `plans/`, `todos/`
- `sessions/`, `session-env/`, `shell-snapshots/`
- `backups/`, `cache/`, `ide/`, `daemon/`, `telemetry/`, `paste-cache/`, `jobs/`, `state/`
- `settings.local.json`, `remote-settings.json`, `mcp-needs-auth-cache.json`

### Unknown

All other entries are unknown. Sync shows them in the plan as `UNKNOWN` and does not link them. A new item type from Claude Code stays unknown until a ccx release puts it in a group.

### Plan and safety

For each shared item, sync uses `lstat` on the path in the account and shows one action:

| Action | Path in the account |
|---|---|
| `LINK` | Missing. |
| `OK` | A link to the correct target. |
| `RELINK` | A link to a different target. Sync removes only the link. |
| `CONFLICT` | A real file or directory where a link must be. |
| `SKIP` | The item is in `own = [...]` of the account. |
| `MKDIR` | `skills/` is missing or is a link. Sync makes a real directory. |

- A CONFLICT item moves to `<dir>/.ccx-backup/<timestamp>/`, with the same relative path. Sync never removes a real file or directory. Sync moves CONFLICT items only with `--force` or after you confirm on a terminal.
- Sync never deletes through a symlink and never reads through a `skills` link.
- Sync refuses an operation when the resolved path is inside the base directory or outside the account directory. It checks all paths before it changes a file.
- Sync never changes the base account.
- `ccx @name` runs a fast health check. It adds missing links and a missing `skills/` directory. It never changes a link or a CONFLICT item.

## Known risk: `plugins/synced` is shared

`plugins/` is linked as one item. Thus, `plugins/synced` is shared too, although Claude Code keys it per organization and account. When two accounts are in different organizations, one account can see or change the synced plugin data of the other account.

To keep `plugins/` separate in one account, put `"plugins"` in `own = [...]` for that account. A per-child link for `plugins/` (as for `skills/`) is a possible follow-up.

## Consequences

- `ccx manage doctor` (#8) uses the same planner and the same groups.
- A new skill in the base directory is available in each account at the next `ccx @name`.
- A user who changes a shared item in an account changes the base file. To keep a different file, use `own = [...]`.

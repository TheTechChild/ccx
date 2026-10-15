# Share the setup with sync

`ccx manage sync` links the setup of the base account `~/.claude` into the other accounts with symbolic links. A change in the base account then shows in all accounts.

```sh
ccx manage sync angel2             # Sync one account.
ccx manage sync --all              # Sync all accounts but the base.
ccx manage sync angel2 --dry-run   # Show the plan only.
ccx manage sync angel2 --force     # Move CONFLICT items to a backup. No prompt.
ccx manage sync --all -y           # Ask no questions.
```

## What sync shares

- One link each: `settings.json`, `CLAUDE.md`, `keybindings.json`, `statusline.py`, `agents/`, `commands/`, `output-styles/`, `plugins/` and each item in `[sync] share`.
- `skills/`: a real directory, with one link for each base skill. Sync never links `skills/synced`.
- Never shared: identity files, credentials, sessions, history and dotfiles.

[ADR 0008](adr/0008-sync-share-lists.md) has the full lists.

> **Known risk:** `plugins/` is one link, so all accounts share `plugins/synced`. To keep `plugins/` separate, put `"plugins"` in `own`.

## The plan

| Action | Meaning |
|---|---|
| `LINK` | Sync makes the missing link. |
| `OK` | The link is correct. |
| `RELINK` | The link has a different target. Sync replaces only the link. |
| `CONFLICT` | A real file or directory is in the place of the link. |
| `SKIP` | The account keeps its own item (`own`). |
| `MKDIR` | `skills/` is missing or a link. Sync makes a real directory. |
| `UNKNOWN` | An unknown base entry, not in `share` or `ignore`. Not shared. |

- Sync moves a CONFLICT item to `<account dir>/.ccx-backup/<timestamp>/` only with `--force` or your confirmation on a terminal. With `-y`, CONFLICT items move only with `--force`.
- Sync never deletes a file or changes the base account. When CONFLICT items stay, sync exits with code 1.
- Sync refuses a change whose real path is in the base directory or outside the account directory.
- Before each launch (`ccx` or `ccx @name`) starts `claude`, ccx adds missing links, for example a new skill. It never changes a CONFLICT item.

## Unknown items

On a terminal, sync first asks: "Select the items to share with all accounts (the others are not shared)". It writes the selected items to `[sync] share` and the others to `[sync] ignore` in `config.toml`, then continues. The write keeps comments and follows a symbolic link. Sync does not ask again about an item in `share` or `ignore`. To change your answer, edit `[sync]`.

With `--dry-run`, `-y` or no terminal, sync does not ask. It shows the `UNKNOWN` items and does not change `config.toml`.

## Keep an item in one account

```toml
[accounts.angel2]
dir = "~/.claude-angel2"
own = ["statusline.py", "skills/tdd"]   # Optional. Can name a [sync] share item.
```

## Copy MCP servers and preferences

Links cannot share the user MCP servers and preferences in each account `.claude.json`. `ccx manage sync --prefs` copies them from the base `~/.claude.json`. It does not make links. It takes `NAME`, `--all` and `--dry-run` like sync. The plan shows names only, never values: `ADD` (a new MCP server), `SKIP` (an MCP server with this name stays unchanged), `SET` (ccx writes the base value of a key) and `OK` (same value).

**MCP servers.** ccx adds them only through `claude mcp add-json -s user <name> '<json>'`, with `CLAUDE_CONFIG_DIR` set to the account directory. ccx never writes `mcpServers` itself. Then ccx shows `MCP servers that need re-auth: ...`: the new `http` and `sse` servers that use OAuth, found from the server config only. ccx never reads or copies MCP OAuth tokens. To log in, run `/mcp` in the account, for example `ccx @angel2`.

**Preferences.** ccx copies only allowlist keys in the base file. It writes the account `.claude.json` to a temporary file and renames it. Other keys stay the same. The built-in allowlist is `theme`, `editorMode`, `verbose`, `autoCompactEnabled`, `preferredNotifChannel`, `autoUpdates`, `diffSidebarOpen`, `deepLinkTerminal`, `claudeInChromeDefaultEnabled`, `hasCompletedOnboarding` and `lastOnboardingVersion`. The onboarding keys prevent a second onboarding. To change the allowlist, use `[prefs]` in `config.toml`:

```toml
[prefs]
allow = ["someKey"]   # Add keys to the allowlist.
deny = ["verbose"]    # Remove keys from the allowlist.
```

ccx never copies `oauthAccount`, `userID`, `machineID`, `mcpServers`, `projects` or keys for caches, counters, usage data and migration state (for example `cached*`, `*Cache`, `*Count`, `numStartups`, `firstStartTime`, `skillUsage`, `pluginUsage`, `tip*`, `migrationVersion`, `*MigrationComplete` and `bridgeOauth*`), even when `allow` names them.

**Running sessions.** ccx does not change an account with a running session: a `sessions/<pid>.json` whose `pid` is a live process with a start time that agrees with `procStart`. A file with a dead pid does not stop the copy. ccx never reads `sessions/*.key`. Stop the session and run the command again.

The exit code is 1 when ccx did not change an account or an MCP server add failed.

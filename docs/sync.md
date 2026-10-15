# Share the setup with sync

`ccx manage sync` links the setup of the base account `~/.claude` into the other accounts with symbolic links. A change in the base account is then available in all accounts.

```sh
ccx manage sync angel2             # Sync one account.
ccx manage sync --all              # Sync all accounts other than the base account.
ccx manage sync angel2 --dry-run   # Show the plan. Change nothing.
ccx manage sync angel2 --force     # Also move the CONFLICT items to a backup, with no prompt.
ccx manage sync --all -y           # Ask no questions.
```

Sync links `settings.json`, `CLAUDE.md`, `keybindings.json`, `statusline.py`, `agents/`, `commands/`, `output-styles/` and `plugins/` as one item each. `skills/` is a real directory in each account, with one link for each base skill. Sync never links `skills/synced`. Sync never links identity files, credentials, sessions, history or dotfiles. [ADR 0008](adr/0008-sync-share-lists.md) has the full lists.

Sync shows a plan with one line for each item:

| Action | Meaning |
|---|---|
| `LINK` | The link is missing. Sync makes it. |
| `OK` | The link is correct. |
| `RELINK` | The link points to a different target. Sync removes only the link and makes a new one. |
| `CONFLICT` | A real file or directory is where the link must be. |
| `SKIP` | The account keeps its own item (`own = [...]`). |
| `MKDIR` | `skills/` is missing or is a link. Sync makes a real directory. |
| `UNKNOWN` | A base entry that ccx does not know and that is not in `[sync] share` or `[sync] ignore`. Sync does not share it. |

On a terminal, sync first asks which `UNKNOWN` items to share: "Select the items to share with all accounts (the others are not shared)". Sync writes the items that you select to `[sync] share` and the other items to `[sync] ignore` in `config.toml`. The write keeps the comments. When `config.toml` is a symbolic link, sync writes the target file. Then sync continues with the new lists. Sync links each `share` item like a built-in shared item. Sync does not ask again about an item in `share` or `ignore`. To change your answer, edit `[sync]` in `config.toml`.

With `--dry-run`, with `-y`, or with no terminal, sync does not ask. It shows the `UNKNOWN` items and does not change `config.toml`. With `-y`, sync also does not ask about CONFLICT items: they move only with `--force`.

Sync moves a CONFLICT item to `<account dir>/.ccx-backup/<timestamp>/` only when you give `--force` or confirm on a terminal. Sync never deletes a file. When CONFLICT items stay, sync exits with code 1. Sync never changes the base account, and it refuses a change whose real path is in the base directory or outside the account directory.

`ccx @name` also does a fast check before it starts `claude`. It adds missing links, for example a new skill. It never changes a CONFLICT item.

To keep an item of your own in one account, add it to `own`:

```toml
[accounts.angel2]
dir = "~/.claude-angel2"
own = ["statusline.py", "skills/tdd"]   # Optional. Sync shows these items as SKIP.
```

`own` can also name an item of `[sync] share`.

> **Known risk:** `plugins/` is one link, so `plugins/synced` is shared by all accounts. To keep `plugins/` separate, put `"plugins"` in `own`.

## Copy MCP servers and preferences

Links cannot share the user MCP servers and the user preferences, because they are in the `.claude.json` file of each account. `ccx manage sync --prefs` copies them from the base `~/.claude.json` into the other accounts. It does not make links.

```sh
ccx manage sync --prefs angel2             # Copy into one account.
ccx manage sync --prefs --all              # Copy into all accounts other than the base account.
ccx manage sync --prefs angel2 --dry-run   # Show the plan. Change nothing.
```

The plan shows names only, never values:

| Action | Meaning |
|---|---|
| `ADD` | The account does not have this MCP server. ccx adds it with `claude mcp add-json -s user`. |
| `SKIP` | The account has an MCP server with this name. ccx does not change or remove it. |
| `SET` | ccx writes the base value of this preference key. |
| `OK` | The account has the same value. |

ccx adds MCP servers only through `claude mcp add-json -s user <name> '<json>'`, with `CLAUDE_CONFIG_DIR` set to the account directory. ccx never writes `mcpServers` itself.

ccx copies only the keys in the allowlist, and only when the key is in the base file. It writes the account `.claude.json` to a temporary file and then renames it. All other keys stay the same. The built-in allowlist is `theme`, `editorMode`, `verbose`, `autoCompactEnabled`, `preferredNotifChannel`, `autoUpdates`, `diffSidebarOpen`, `deepLinkTerminal`, `claudeInChromeDefaultEnabled`, `hasCompletedOnboarding` and `lastOnboardingVersion`. The onboarding keys prevent a second onboarding in the new account. To change the allowlist, use `[prefs]` in `config.toml`:

```toml
[prefs]
allow = ["someKey"]   # Add keys to the allowlist.
deny = ["verbose"]    # Remove keys from the allowlist.
```

ccx never copies some keys, also when `allow` names them: `oauthAccount`, `userID`, `machineID`, `mcpServers`, `projects`, and the keys for caches, counters, usage data and migration state (for example `cached*`, `*Cache`, `*Count`, `numStartups`, `firstStartTime`, `skillUsage`, `pluginUsage`, `tip*`, `migrationVersion`, `*MigrationComplete` and `bridgeOauth*`).

ccx does not change an account that has a running session. It reads the `sessions/<pid>.json` files of the account. A session is running when its `pid` is a live process and the start time of the process agrees with `procStart`. A file with a dead pid does not stop the copy. ccx never reads the `sessions/*.key` files. When a session is running, stop it and run the command again. The exit code is 1 when ccx did not change an account or an MCP server add failed.

After the copy, ccx shows `MCP servers that need re-auth: ...`. These are the new `http` and `sse` servers that use OAuth. ccx finds them from the server config only. ccx never reads or copies MCP OAuth tokens. To log in to these servers, run `/mcp` in the account, for example in `ccx @angel2`.

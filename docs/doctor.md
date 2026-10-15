# Check the setup with doctor

`ccx manage doctor` checks the ccx setup. It shows one line for each check with `OK`, `INFO`, `WARN` or `FAIL`. `INFO` is a note, not a problem. Doctor only reports. It never changes a file.

```sh
ccx manage doctor          # Show the results as text.
ccx manage doctor --json   # Show the results as JSON for scripts.
```

| Check | What doctor examines |
|---|---|
| `config` | The config loads. |
| `claude` | `claude` is on `PATH`. Doctor shows its version. |
| `env` | `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN`, `CLAUDE_SECURESTORAGE_CONFIG_DIR` and `CLAUDE_CONFIG_DIR` in the current shell. ccx removes or replaces them at launch. Doctor never shows their values. |
| `path` | Each account directory exists, and the config writes it in the canonical form (absolute, with no trailing slash). |
| `stray` | The stray file `~/.claude/.claude.json`. |
| `unknown` | `INFO` for each entry in `~/.claude` that is not on the share list, not on the never-share list, and not in `[sync] share` or `[sync] ignore`. Run `ccx manage sync --all` on a terminal to select which of these entries to share. |
| `link` | The sync plan of each account. A shared item that is a real file or directory, for example `settings.json` after a write replaced the link, is a `FAIL`. |
| `skill` | Real directories and files in `<account>/skills/` that are not links and not `synced`: "local-only skill <name> in <account>; promote to base?". |
| `home-path` | `hooks` and `statusLine` commands in `settings.json` that use the absolute home path. Doctor shows the `$HOME/...` form. |
| `service` | The installed background service. Each path in the job (ccx and `claude`) exists. If not, doctor tells you to run `ccx manage service install` again. Doctor shows no line when no service is installed. |
| `keychain` | macOS only. `INFO` for each keychain item `Claude Code-credentials-*` that agrees with no account directory. Doctor reads only the item names, never the secret values. |

The last line counts the levels, for example `9 OK, 2 INFO, 0 WARN, 0 FAIL`. The exit code is 0 when all checks pass or give only `INFO`, 1 when there is a `WARN` and 2 when there is a `FAIL`. In the `--json` output, the level of a note is `"INFO"`.

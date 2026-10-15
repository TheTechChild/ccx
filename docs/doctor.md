# Check the setup with doctor

`ccx manage doctor` checks the ccx setup. It shows one line per check: `OK`, `INFO` (a note, not a problem), `WARN` or `FAIL`. It changes no file.

```sh
ccx manage doctor          # Text.
ccx manage doctor --json   # JSON for scripts.
```

| Check | Examines |
|---|---|
| `config` | The config loads. |
| `claude` | `claude` is on `PATH`. Doctor shows its version. |
| `env` | `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN`, `CLAUDE_SECURESTORAGE_CONFIG_DIR` and `CLAUDE_CONFIG_DIR` in the shell. ccx removes or replaces them at launch. Doctor hides their values. |
| `path` | Each account directory exists and is canonical in the config: absolute, no trailing slash. |
| `stray` | A stray `~/.claude/.claude.json`. |
| `unknown` | `INFO` for each `~/.claude` entry that is not on the share or never-share list, `[sync] share` or `[sync] ignore`. Run `ccx manage sync --all` on a terminal to select them. |
| `link` | The sync plan of each account. A shared item that is a real file or directory is a `FAIL`, for example `settings.json` after a write replaced the link. |
| `skill` | Directories and files in `<account>/skills/` that are not links and not `synced`: "local-only skill <name> in <account>; promote to base?". |
| `home-path` | `hooks` and `statusLine` commands in `settings.json` that use the absolute home path. Doctor shows the `$HOME/...` form. |
| `service` | Each path (ccx and `claude`) in the installed background service job exists. If not, it says to run `ccx manage service install` again. No service gives no line. |
| `keychain` | macOS only. `INFO` for each keychain item `Claude Code-credentials-*` that agrees with no account directory. It reads only names, not secrets. |

The last line counts the levels, for example `9 OK, 2 INFO, 0 WARN, 0 FAIL`. Exit code: 0 for only `OK` or `INFO`, 1 for a `WARN` and 2 for a `FAIL`. In `--json`, a note is `"INFO"`.

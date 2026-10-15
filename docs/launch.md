# Start Claude Code

`ccx` starts `claude` on one account: the account that you name, or the account that ccx selects from the usage.

```sh
ccx @angel2 --model opus   # Use angel2. Send "--model opus" to claude.
ccx                        # Use CCX_ACCOUNT, else automatic selection.
ccx -- status              # Send all arguments to claude, also "@..." and ccx words.
```

The first words `manage`, `status`, `usage` and `handoff` are ccx commands. ccx sends other arguments to `claude` unchanged. If ccx cannot start `claude`, it exits with code 127.

## Automatic selection

Without `@name` or `CCX_ACCOUNT`, ccx selects an enabled account from the usage cache, or `cachedUsageUtilization` in `.claude.json` when it is newer ([Automatic refresh](usage-data.md#automatic-refresh)).

1. An account is "limited" at session usage 90% or more, weekly usage 95% or more, or matched scoped weekly usage 98% or more. A scoped window matches when the session model name (`--model`, else `model` in `settings.json`) contains its model name, in any case. A window with a past reset time counts as 0%.
2. A not-limited account scores (100 − the higher of session % and weekly %) × weight. Default weights: Pro 1, Max 5x 5, Max 20x 20, else 5 (Team, Enterprise, unknown). The tier comes from `oauthAccount` in `.claude.json`. An account `weight` key overrides it.
3. Unknown usage scores as 50% used and never makes an account limited. Known usage wins a tie.
4. ccx stays on the last account (from `~/.local/state/ccx/state.json`) when it is not limited and scores within 10 points of the best (0-100 scale).
5. When all accounts are limited, ccx starts the account with the soonest reset, with a warning, for example `all accounts limited; angel2 frees 14:05 (37m)`.
6. ccx selects an [API-key account](api-key-accounts.md) only when all subscription accounts are limited, and asks first.

Before ccx starts `claude`, it writes the selected account to `state.json` at mode 0600 and shows one banner line on stderr, for example `ccx → angel2 · 5h 23% · wk 61% (resets Thu 09:00) · /usage 4m`.

## Claude subcommands

Some `claude` subcommands change one account, such as `auth`, `mcp`, `plugin` and `setup-token`. For these, ccx ignores usage and uses `@name` (`ccx @angel2 mcp add ...`), else `CCX_ACCOUNT` (`CCX_ACCOUNT=angel2 ccx plugin list`), else the base account `~/.claude` with `CLAUDE_CONFIG_DIR` unset (`ccx auth status`), with one note on stderr.

Only the first `claude` argument can be a subcommand: `ccx -p "auth"` is a normal launch, and `ccx -- auth status` is the same as `ccx auth status`. The list is `CLAUDE_SUBCOMMANDS` in `src/launch.rs`, from `Commands:` in `claude --help`. `config` is not a `claude` subcommand.

## Environment

| Variable | Use |
|---|---|
| `CCX_ACCOUNT` | The account when there is no `@name`. |
| `CCX_ACTIVE` | Output: the account name. ccx does not read it. |
| `CCX_CLAUDE_BIN` | The path of `claude`, when `launch.claude_bin` is not set. |
| `CCX_CONFIG` | The path of `config.toml`. |
| `CCX_DEPTH` | Internal. ccx stops with an error when `claude` starts ccx too many times. |
| `CCX_DRY_RUN=1` | Show the account, environment changes and arguments. Do not start `claude`. |
| `CCX_SERVICE_KIND` | For tests: the service manager of `ccx manage service`: `launchd`, `systemd` or `cron`. |
| `CCX_NO_REFRESH=1` | No usage fetch or background refresh on an automatic launch. |

Before ccx starts `claude`, it removes `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN` and `CLAUDE_SECURESTORAGE_CONFIG_DIR`, because they override the account login, and shows one warning for each. For a paid launch on an API-key account, ccx then sets `ANTHROPIC_API_KEY` to the key of that account.

ccx shows no banner, notes or warnings when stdout or stderr is not a terminal, or for `-p` runs.

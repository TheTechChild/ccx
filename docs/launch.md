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
2. ccx fills the accounts in order. It uses the first account until that account is limited, then the next account. The order is fixed, and you do not set it:
   1. The base account (`~/.claude`).
   2. The other subscription accounts, in name order.
   3. The [API-key accounts](api-key-accounts.md), always last.
3. ccx selects the first subscription account in the order that is not limited. When a window of an earlier account resets, the next launch goes back to that account.
4. Unknown usage never makes an account limited. Running sessions do not change the order ([ADR 0014](adr/0014-fill-accounts-in-order.md)).
5. When all accounts are limited, ccx starts the account with the soonest reset, with a warning, for example `all accounts limited; angel2 frees 14:05 (37m)`.
6. ccx selects an [API-key account](api-key-accounts.md) only when all subscription accounts are limited, and asks first.

**Limit hits.** When the newest session in the cwd stopped at a usage limit (the last message is the limit error of Claude Code), that limit is newer data than the usage cache. ccx counts that window of the account as 100% until its reset time. When ccx then selects a different account, it shows a note: `ccx: the last session here stopped at the angel2 usage limit. To continue it on a different account, run ccx -c.` ccx ignores a hit that a handoff copied into a different account.

Before ccx starts `claude`, it writes the selected account to `~/.local/state/ccx/state.json` at mode 0600 and shows one banner line on stderr, for example `ccx → angel2 · 5h 23% · wk 61% (resets Thu 09:00) · 1 running · /usage 4m`. The banner shows the running sessions only when there are some.

## Named accounts

`ccx @name` always starts the named account. When that account is limited, ccx shows a warning on a terminal, for example `ccx: warning: angel2 is limited (5h 100%); it frees 17:29 (56m). Run ccx with no @name to pick an account.` The check uses the usage data and a limit hit at the end of the session that `claude` opens (`--resume <id>`), else the newest session of the account in the cwd. It reads only local files.

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

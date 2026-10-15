# Start Claude Code

```sh
ccx @angel2 --model opus   # Start claude as account angel2. ccx sends "--model opus" to claude.
ccx                        # Start claude as the account in CCX_ACCOUNT, or as the account that ccx selects.
ccx -- status              # Send all arguments to claude. Use this when an argument starts with "@" or is a ccx word.
```

ccx keeps these first words for its own commands: `manage`, `status`, `usage` and `handoff`. ccx sends all other arguments to `claude` with no change.

## Automatic selection

When you do not give `@name` and `CCX_ACCOUNT` is not set, ccx selects an enabled account from the usage data. ccx uses the usage cache, or `cachedUsageUtilization` in `.claude.json` when it is newer. Refer to [Automatic refresh](usage-data.md#automatic-refresh) for how ccx keeps the cache fresh.

1. An account is "limited" when its session usage is 90% or more, its weekly usage is 95% or more, or a matched scoped weekly usage is 98% or more. A scoped weekly window matches when its model name is in the session model name (`--model`, else `model` in `settings.json`). Case is not important. A window whose reset time is in the past counts as 0%.
2. The score of an account that is not limited is (100 − the higher of the session % and the weekly %) × the weight. The default weights are Pro 1, Max 5x 5 and Max 20x 20. Team, Enterprise and unknown tiers use 5. ccx gets the tier from `oauthAccount` in `.claude.json`. The `weight` key of an account overrides the default.
3. When the usage of an account is not known, ccx scores it as 50% used. Unknown usage never makes an account limited. On a tie, an account with known usage is first.
4. ccx stays on the last account (from `~/.local/state/ccx/state.json`) when it is not limited and its score is within 10 points of the best score on a 0-100 scale.
5. When all accounts are limited, ccx starts the account with the soonest reset and shows a warning, for example `all accounts limited; angel2 frees 14:05 (37m)`.
6. Automatic selection never selects an [API-key account](api-key-accounts.md) while a subscription account is not limited. When all subscription accounts are limited, ccx asks before it uses a paid account.

Before ccx starts `claude`, it shows one banner line on stderr, for example `ccx → angel2 · 5h 23% · wk 61% (resets Thu 09:00) · /usage 4m`. ccx does not show the banner or the warnings when stdout or stderr is not a terminal, or for `-p` runs. ccx writes the selected account to `state.json` at mode 0600.

## Claude subcommands

Some `claude` subcommands change one account, for example `auth`, `mcp`, `plugin` and `setup-token`. ccx never selects an account for these subcommands by usage. ccx uses this order:

1. `@name`: `ccx @angel2 mcp add ...` runs the subcommand in account `angel2`.
2. `CCX_ACCOUNT`: `CCX_ACCOUNT=angel2 ccx plugin list` runs the subcommand in account `angel2`.
3. The base account: `ccx auth status` runs the subcommand in `~/.claude`, with `CLAUDE_CONFIG_DIR` unset. When stderr is a terminal, ccx shows one note on stderr.

ccx finds a subcommand only in the first `claude` argument. `ccx -p "auth"` is a normal launch. `ccx -- auth status` follows the same rule as `ccx auth status`. The list of subcommands is `CLAUDE_SUBCOMMANDS` in `src/launch.rs`. It comes from the `Commands:` section of `claude --help`. `config` is not a `claude` subcommand.

## Environment

| Variable | Use |
|---|---|
| `CCX_ACCOUNT` | Input. Select an account when you do not give `@name`. |
| `CCX_ACTIVE` | Output. ccx sets it to the name of the account. ccx does not read it. |
| `CCX_CLAUDE_BIN` | Input. The path of `claude`, when `launch.claude_bin` is not set. |
| `CCX_CONFIG` | Input. The path of `config.toml`. |
| `CCX_DEPTH` | Internal. ccx stops with an error when `claude` starts ccx again too many times. |
| `CCX_DRY_RUN=1` | Input. Show the account, the environment changes and the arguments. Do not start `claude`. |
| `CCX_SERVICE_KIND` | Input, for tests. Select the service manager of `ccx manage service`: `launchd`, `systemd` or `cron`. |
| `CCX_NO_REFRESH=1` | Input. An automatic launch does not fetch usage and does not start the background refresh. |

ccx removes `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN` and `CLAUDE_SECURESTORAGE_CONFIG_DIR` before it starts `claude`, because they override the account login. For a paid launch on an API-key account, ccx then sets `ANTHROPIC_API_KEY` to the key of that account. On a terminal, ccx shows one warning for each variable that it removes. When stdout or stderr is not a terminal, or for `-p` runs, ccx shows no banner and no warnings.

If ccx cannot start `claude`, it exits with code 127.

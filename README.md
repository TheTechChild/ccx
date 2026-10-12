# ccx

ccx is a command-line tool. It starts Claude Code on the account that has the most usage available.

> **Status:** Work in progress. ccx starts Claude Code on an account that you name. Automatic selection is not available yet. Refer to [PLAN.md](PLAN.md) for the plan.

## Usage

```sh
ccx @angel2 --model opus   # Start claude as account angel2. ccx sends "--model opus" to claude.
ccx                        # Start claude as the account in CCX_ACCOUNT, or as the base account.
ccx -- status              # Send all arguments to claude. Use this when an argument starts with "@" or is a ccx word.
```

ccx keeps these first words for its own commands: `manage`, `status`, `usage` and `handoff`. ccx sends all other arguments to `claude` with no change.

### Claude subcommands

Some `claude` subcommands change one account, for example `auth`, `mcp`, `plugin` and `setup-token`. ccx never selects an account for these subcommands by usage. ccx uses this order:

1. `@name`: `ccx @angel2 mcp add ...` runs the subcommand in account `angel2`.
2. `CCX_ACCOUNT`: `CCX_ACCOUNT=angel2 ccx plugin list` runs the subcommand in account `angel2`.
3. The base account: `ccx auth status` runs the subcommand in `~/.claude`, with `CLAUDE_CONFIG_DIR` unset. When stderr is a terminal, ccx shows one note on stderr.

ccx finds a subcommand only in the first `claude` argument. `ccx -p "auth"` is a normal launch. `ccx -- auth status` follows the same rule as `ccx auth status`. The list of subcommands is `CLAUDE_SUBCOMMANDS` in `src/launch.rs`. It comes from the `Commands:` section of `claude --help`. `config` is not a `claude` subcommand.

## Configuration

ccx reads `config.toml` from the first location that applies:

1. `$CCX_CONFIG`
2. `$XDG_CONFIG_HOME/ccx/config.toml`
3. `~/.config/ccx/config.toml` (also on macOS)

```toml
[accounts.main]
dir = "~/.claude"          # The base account. Exactly one account must use this directory.

[accounts.angel2]
dir = "~/.claude-angel2"
```

Put email addresses only in the overlay file `config.local.toml`. ccx reads it from `$XDG_CONFIG_HOME/ccx/`, or from `~/.config/ccx/`. ccx merges the overlay over `config.toml`. Do not commit the overlay. Give it mode 0600.

```toml
[accounts.angel2]
email = "work@example.com"
```

Issue #3 has the full configuration schema.

## Environment

| Variable | Use |
|---|---|
| `CCX_ACCOUNT` | Input. Select an account when you do not give `@name`. |
| `CCX_ACTIVE` | Output. ccx sets it to the name of the account. ccx does not read it. |
| `CCX_CLAUDE_BIN` | Input. The path of `claude`, when `launch.claude_bin` is not set. |
| `CCX_CONFIG` | Input. The path of `config.toml`. |
| `CCX_DEPTH` | Internal. ccx stops with an error when `claude` starts ccx again too many times. |
| `CCX_DRY_RUN=1` | Input. Show the account, the environment changes and the arguments. Do not start `claude`. |

ccx removes `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN` and `CLAUDE_SECURESTORAGE_CONFIG_DIR` before it starts `claude`, because they override the account login. On a terminal, ccx shows one warning for each variable that it removes. When stdout or stderr is not a terminal, ccx shows no banner and no warnings.

If ccx cannot start `claude`, it exits with code 127.

## How ccx works

- Each account has its own Claude Code configuration directory. The base account uses `~/.claude`.
- ccx sets `CLAUDE_CONFIG_DIR` for the account that it selects, and then starts `claude`.
- ccx does not read, keep or refresh Claude Code credentials. All logins go through `claude auth login`.

## Development

This project uses [mise](https://mise.jdx.dev) for all tools.

1. Install the tools: `mise install`.
2. Run all checks: `mise run ci`.

The checks are `fmt`, `lint`, `test` and `check-emails`. You can run each check with `mise run <name>`.

## Privacy rule

Do not put a real email address in this repository. Use `@example.com` addresses in code, tests and docs. The `check-emails` check fails on all other addresses, except addresses that contain `noreply`.

## License

MIT. Refer to [LICENSE](LICENSE).

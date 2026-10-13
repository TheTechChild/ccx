# ccx

ccx is a command-line tool. It starts Claude Code on the account that has the most usage available.

> **Status:** Work in progress. ccx starts Claude Code on an account that you name. `ccx manage adopt`, `ccx manage list`, `ccx manage shell-init`, `ccx manage sync`, `ccx manage doctor`, `ccx status` and `ccx usage refresh` are available. Automatic selection is not available yet. Refer to [PLAN.md](PLAN.md) for the plan.

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

## Manage accounts

### Adopt the accounts on this machine

```sh
ccx manage adopt      # Show the plan, then ask for confirmation.
ccx manage adopt -y   # Do not ask. Use this in scripts.
```

`ccx manage adopt` finds `~/.claude` (the base account, name `main`) and each `~/.claude-<name>` directory. For each directory, it runs `claude auth status --json` to get the email. For the base account, `CLAUDE_CONFIG_DIR` is not set. For the other accounts, `CLAUDE_CONFIG_DIR` is the directory of the account.

Adopt shows the plan and writes nothing until you confirm. When there is no terminal and you do not give `-y`, adopt stops and writes nothing.

After you confirm, adopt does these steps:

1. It adds each new account to `config.toml` with its `dir`. `config.toml` gets no email addresses.
2. It writes the email addresses only to `config.local.toml`, with mode 0600.
3. It moves the stray files `~/.claude/.claude.json` and `~/.claude/.claude.json.lock` to a backup directory and shows the path. Adopt never deletes these files. The backup directory is `$XDG_STATE_HOME/ccx/backup/adopt-<time>`, or `~/.local/state/ccx/backup/adopt-<time>`.

Adopt keeps the other entries and the comments in a file that exists. It writes each file to a temporary file and then renames it. When `config.toml` is a symlink, adopt keeps the symlink and changes the target file. Adopt always writes `config.local.toml` at the overlay path, never next to the symlink target.

You can run adopt again. It adds only the accounts and the emails that are not in the config.

### List the accounts

```sh
ccx manage list          # Show a table: name, directory, email and aliases.
ccx manage list --json   # Show stable JSON for scripts.
```

### Add completions and aliases to your shell

`ccx manage shell-init` prints shell code. Add one line to your shell profile:

```sh
eval "$(ccx manage shell-init zsh)"     # ~/.zshrc. Put this line after compinit.
eval "$(ccx manage shell-init bash)"    # ~/.bashrc
ccx manage shell-init fish | source     # ~/.config/fish/config.fish
```

The output has two parts:

1. Completions for `ccx`: the commands, the `manage` subcommands and one `@name` token for each account.
2. One alias for each name in `aliases = [...]` in `config.toml`. For example, `aliases = ["claude2"]` in account `angel2` gives `alias claude2='ccx @angel2'`.

Shell-init never makes an alias or a function for `claude`. The bare `claude` command stays on the base account. An alias name must be a shell identifier: letters, digits and `_`, and not a digit first. Shell-init also refuses the names `claude` and `ccx`, and an alias name that two accounts use. When an alias name is not valid, shell-init prints an error and no shell code, and exits with a non-zero code. When `config.toml` does not exist, shell-init prints only the completions.

#### Replace a hand-written alias

If your profile has a hand-written alias, for example `alias claude2='CLAUDE_CONFIG_DIR=~/.claude-angel2 claude'`, do these steps:

1. Add the alias name to the account in `config.toml`: `aliases = ["claude2"]`.
2. Add the shell-init line to your shell profile.
3. Remove the hand-written alias from your profile.
4. Start a new shell. Make sure that `alias claude2` shows `ccx @angel2`, and that `type claude` shows the `claude` program, not an alias.

### Share the setup with sync

`ccx manage sync` links the setup of the base account `~/.claude` into the other accounts with symbolic links. A change in the base account is then available in all accounts.

```sh
ccx manage sync angel2             # Sync one account.
ccx manage sync --all              # Sync all accounts other than the base account.
ccx manage sync angel2 --dry-run   # Show the plan. Change nothing.
ccx manage sync angel2 --force     # Also move the CONFLICT items to a backup, with no prompt.
```

Sync links `settings.json`, `CLAUDE.md`, `keybindings.json`, `statusline.py`, `agents/`, `commands/`, `output-styles/` and `plugins/` as one item each. `skills/` is a real directory in each account, with one link for each base skill. Sync never links `skills/synced`. Sync never links identity files, credentials, sessions, history or dotfiles. [ADR 0008](docs/adr/0008-sync-share-lists.md) has the full lists.

Sync shows a plan with one line for each item:

| Action | Meaning |
|---|---|
| `LINK` | The link is missing. Sync makes it. |
| `OK` | The link is correct. |
| `RELINK` | The link points to a different target. Sync removes only the link and makes a new one. |
| `CONFLICT` | A real file or directory is where the link must be. |
| `SKIP` | The account keeps its own item (`own = [...]`). |
| `MKDIR` | `skills/` is missing or is a link. Sync makes a real directory. |
| `UNKNOWN` | A base entry that ccx does not know. Sync does not share it. |

Sync moves a CONFLICT item to `<account dir>/.ccx-backup/<timestamp>/` only when you give `--force` or confirm on a terminal. Sync never deletes a file. When CONFLICT items stay, sync exits with code 1. Sync never changes the base account, and it refuses a change whose real path is in the base directory or outside the account directory.

`ccx @name` also does a fast check before it starts `claude`. It adds missing links, for example a new skill. It never changes a CONFLICT item.

To keep an item of your own in one account, add it to `own`:

```toml
[accounts.angel2]
dir = "~/.claude-angel2"
own = ["statusline.py", "skills/tdd"]   # Optional. Sync shows these items as SKIP.
```

> **Known risk:** `plugins/` is one link, so `plugins/synced` is shared by all accounts. To keep `plugins/` separate, put `"plugins"` in `own`.

### Check the setup with doctor

`ccx manage doctor` checks the ccx setup. It shows one line for each check with `OK`, `WARN` or `FAIL`. Doctor only reports. It never changes a file.

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
| `unknown` | Entries in `~/.claude` that are not on the share list or the never-share list. |
| `link` | The sync plan of each account. A shared item that is a real file or directory, for example `settings.json` after a write replaced the link, is a `FAIL`. |
| `skill` | Real directories and files in `<account>/skills/` that are not links and not `synced`: "local-only skill <name> in <account>; promote to base?". |
| `home-path` | `hooks` and `statusLine` commands in `settings.json` that use the absolute home path. Doctor shows the `$HOME/...` form. |
| `keychain` | macOS only. Keychain items `Claude Code-credentials-*` that agree with no account directory. Doctor reads only the item names, never the secret values. |

The exit code is 0 when all checks pass, 1 when there is a `WARN` and 2 when there is a `FAIL`.

## Usage data

### Show the status of each account

```sh
ccx status          # One row for each account.
ccx status --json   # The same data as stable JSON.
```

```text
NAME    EMAIL             KIND  TIER     SESSION      WEEKLY      SCOPED                   SOURCE  AGE
angel2  work@example.com  max   Max 5x   1% (4h08m)   0% (6d6h)   ExampleModel 0% (6d6h)   /usage  0s
main    -                 max   Max 20x  11% (2h48m)  21% (3d2h)  ExampleModel 12% (3d2h)  /usage  2m
```

- `EMAIL` comes from `config.local.toml`. If there is no email, ccx shows `-`.
- `KIND` is `subscriptionType` from `claude auth status`. `TIER` comes from `oauthAccount.organizationRateLimitTier` in the `.claude.json` of the account. If ccx does not know the tier, it shows the raw value.
- `SESSION`, `WEEKLY` and `SCOPED` show the percent used. The time in parentheses is the time until the window resets.
- `SOURCE` is `/usage` or `cachedUsageUtilization`. `AGE` is the age of the usage data.

ccx uses the usage cache when it is less than `usage.fresh_minutes` old (default 15). If not, ccx runs the `/usage` fetch for each account at the same time, and waits for a maximum of `usage.fetch_timeout_secs` (default 5). If the fetch fails, ccx uses `cachedUsageUtilization` from the `.claude.json` of the account. If the two sources are missing, ccx shows `unknown`. A failed fetch never makes the command fail.

The exit code is:

| Code | Meaning |
|---|---|
| 0 | At least one enabled account is logged in and not limited. |
| 3 | All logged-in enabled accounts are limited: session usage is `pick.session_avoid` (default 90) or more, or weekly usage is `pick.weekly_avoid` (default 95) or more. |
| 4 | No enabled account is logged in. |

`ccx status` has no session model. Thus, a scoped weekly window does not make an account limited here. Unknown usage does not make an account limited.

### Refresh the usage cache

```sh
ccx usage refresh angel2   # Fetch the usage of one account.
ccx usage refresh --all    # Fetch the usage of all enabled accounts.
```

The refresh writes `~/.local/state/ccx/usage/<name>.json`. ccx writes a temporary file at mode 0600, then renames it. The refresh waits for a maximum of 30 seconds for each account. The exit code is 1 when a fetch fails.

### The `/usage` fetch

ccx runs this command for each account ([ADR 0005](docs/adr/0005-m0-usage-hooks-off.md)):

```sh
claude -p "/usage" --output-format stream-json --verbose --no-session-persistence --strict-mcp-config --settings '{"disableAllHooks":true}'
```

- The command always runs in `~/.local/state/ccx/probe`. ccx makes this directory at mode 0700 if it does not exist. The command never runs in your current directory. Thus, project settings, project hooks, project MCP servers and project `CLAUDE.md` files have no effect.
- For the base account, `CLAUDE_CONFIG_DIR` is not set. For the other accounts, `CLAUDE_CONFIG_DIR` is the directory of the account.
- All hooks are off. `--no-session-persistence` stops the session file.
- ccx reads `usage_report.rate_limits.limits[]` from the `assistant` event.
- ccx never reads, keeps or refreshes Claude Code credentials. A test fails if the code uses the OAuth token endpoint, reads `.credentials.json` or reads a keychain secret.

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
aliases = ["claude2"]      # Optional. `ccx manage shell-init` makes `alias claude2='ccx @angel2'`.
own = ["statusline.py"]    # Optional. Items that `ccx manage sync` does not link.
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

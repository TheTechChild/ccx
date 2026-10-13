# ccx

ccx is a command-line tool. It starts Claude Code on the account that has the most usage available.

ccx is for a person who has more than one Claude Code account, for example a personal Max account and a work account. Each account has its own configuration directory. ccx reads the usage of each account and starts `claude` on the account that has the most capacity available. You can also name the account.

> **Status:** Version 0.1.0. `ccx @name`, automatic selection, `ccx status`, `ccx usage refresh`, `ccx manage adopt`, `ccx manage add`, `ccx manage list`, `ccx manage shell-init`, `ccx manage sync`, `ccx manage doctor`, `ccx manage service`, `ccx handoff` and the automatic handoff on `ccx -c` and `ccx --resume <id>` are available. Refer to [PLAN.md](PLAN.md) for the plan.

## Credential policy

ccx never reads, stores or refreshes Claude Code credentials. ccx does not read `.credentials.json` or the keychain secrets. All logins go through `claude auth login`. ccx gets the usage data from the `claude` program. Refer to [ADR 0002](docs/adr/0002-no-credential-access.md).

## Install

Install ccx with [mise](https://mise.jdx.dev):

```sh
mise use -g github:TheTechChild/ccx
ccx --version
```

mise downloads the release archive for your system from GitHub and puts `ccx` on `PATH`. The releases are for macOS and Linux, on x86_64 (amd64) and arm64 (aarch64). Each release has a `sha256.sum` file with the checksums of the archives.

You must also have Claude Code (`claude`) on `PATH`.

Each archive also has the static completion files in `completions/`: `_ccx` (zsh), `ccx.bash` and `ccx.fish`. These files do not complete the account names. To get the account names and the aliases, use `ccx manage shell-init` (refer to [Add completions and aliases to your shell](#add-completions-and-aliases-to-your-shell)).

## Quick start

1. Log in to each account with `claude auth login`. The base account uses `~/.claude`. Each other account uses its own directory, for example `CLAUDE_CONFIG_DIR=~/.claude-angel2 claude auth login`.
2. Add the accounts to the ccx config: `ccx manage adopt`.
3. Add completions and aliases to your shell profile: `eval "$(ccx manage shell-init zsh)"`. For example, `aliases = ["claude2"]` in account `angel2` gives `alias claude2='ccx @angel2'`.
4. Start Claude Code:
   - `ccx` selects the account that has the most capacity available.
   - `ccx @angel2` starts account `angel2`.
5. Show the usage of each account: `ccx status`.

The bare `claude` command stays on the base account. ccx never makes an alias for `claude`.

## Usage

```sh
ccx @angel2 --model opus   # Start claude as account angel2. ccx sends "--model opus" to claude.
ccx                        # Start claude as the account in CCX_ACCOUNT, or as the account that ccx selects.
ccx -- status              # Send all arguments to claude. Use this when an argument starts with "@" or is a ccx word.
```

ccx keeps these first words for its own commands: `manage`, `status`, `usage` and `handoff`. ccx sends all other arguments to `claude` with no change.

### Automatic selection

When you do not give `@name` and `CCX_ACCOUNT` is not set, ccx selects an enabled account from the usage data. ccx uses the usage cache, or `cachedUsageUtilization` in `.claude.json` when it is newer. Refer to [Automatic refresh](#automatic-refresh) for how ccx keeps the cache fresh.

1. An account is "limited" when its session usage is 90% or more, its weekly usage is 95% or more, or a matched scoped weekly usage is 98% or more. A scoped weekly window matches when its model name is in the session model name (`--model`, else `model` in `settings.json`). Case is not important. A window whose reset time is in the past counts as 0%.
2. The score of an account that is not limited is (100 − the higher of the session % and the weekly %) × the weight. The default weights are Pro 1, Max 5x 5 and Max 20x 20. Team, Enterprise and unknown tiers use 5. ccx gets the tier from `oauthAccount` in `.claude.json`. The `weight` key of an account overrides the default.
3. When the usage of an account is not known, ccx scores it as 50% used. Unknown usage never makes an account limited. On a tie, an account with known usage is first.
4. ccx stays on the last account (from `~/.local/state/ccx/state.json`) when it is not limited and its score is within 10 points of the best score on a 0-100 scale.
5. When all accounts are limited, ccx starts the account with the soonest reset and shows a warning, for example `all accounts limited; angel2 frees 14:05 (37m)`.

Before ccx starts `claude`, it shows one banner line on stderr, for example `ccx → angel2 · 5h 23% · wk 61% (resets Thu 09:00) · /usage 4m`. ccx does not show the banner or the warnings when stdout or stderr is not a terminal, or for `-p` runs. ccx writes the selected account to `state.json` at mode 0600.

### Claude subcommands

Some `claude` subcommands change one account, for example `auth`, `mcp`, `plugin` and `setup-token`. ccx never selects an account for these subcommands by usage. ccx uses this order:

1. `@name`: `ccx @angel2 mcp add ...` runs the subcommand in account `angel2`.
2. `CCX_ACCOUNT`: `CCX_ACCOUNT=angel2 ccx plugin list` runs the subcommand in account `angel2`.
3. The base account: `ccx auth status` runs the subcommand in `~/.claude`, with `CLAUDE_CONFIG_DIR` unset. When stderr is a terminal, ccx shows one note on stderr.

ccx finds a subcommand only in the first `claude` argument. `ccx -p "auth"` is a normal launch. `ccx -- auth status` follows the same rule as `ccx auth status`. The list of subcommands is `CLAUDE_SUBCOMMANDS` in `src/launch.rs`. It comes from the `Commands:` section of `claude --help`. `config` is not a `claude` subcommand.

### Move a session to a different account

When one account reaches its limit, `ccx handoff` copies a session to a different account and resumes it there.

```sh
ccx handoff                       # List the threads for this directory from all accounts. On a terminal, select a thread and the target account.
ccx handoff --all                 # List the threads from all directories.
ccx handoff <id> @angel2          # Copy session <id> into angel2, then run `claude --resume <id>` in angel2. There are no prompts.
ccx handoff <id> @angel2 --no-resume   # Copy only. Print the session id in the target account.
```

The list shows the session id, the account, the time of the last change and the first prompt. When stdin or stderr is not a terminal, `ccx handoff` prints the list and stops. For `<id>`, you can give a unique part of 8 or more characters at the start of the id.

Handoff copies these files of the session from the source account into the target account:

- `projects/<dir>/<id>.jsonl` (the transcript). `<dir>` is the encoded name of the session directory.
- Each `projects/*/<id>/` directory (the sidecar directories).
- `file-history/<id>/` and `session-env/<id>/`, if they are present.
- `plans/<slug>.md` for each `slug` in the transcript, if the file is present.

Handoff does not copy `todos/`, `projects/<dir>/memory/` or `jobs/`. Handoff never moves, deletes or changes a file in the source account.

Handoff refuses when the session is open:

- A `sessions/<pid>.json` file in the source or the target account has the session id, the process is alive, and its start time agrees with `procStart`. Handoff never reads the `sessions/*.key` files.
- The transcript changed in the last 5 seconds.

When the target has a copy of the session already:

- If the copy in the target is a prefix of the source transcript, handoff overwrites it.
- If not, handoff does not change the copy in the target. It writes a fork with a new session id, and shows the new id. Refer to [ADR 0009](docs/adr/0009-handoff-fork-by-rewrite.md).

When `oauthAccount.organizationUuid` in `.claude.json` is different in the two accounts, handoff shows a warning before the copy: Claude cannot use the earlier thinking in the target account, and the next responses can be slower and use more tokens.

After the copy, ccx runs `claude --resume <id>` in the target account from the directory of the session. It uses the same environment rules as `ccx @name`.

### Automatic handoff on `ccx -c` and `ccx --resume <id>`

`ccx -c`, `ccx --continue` and `ccx --resume <id>` (also `-r <id>` and `--resume=<id>`) find the account that owns the session from the files on disk. ccx also finds sessions that you started with plain `claude` or with an alias such as `claude2`.

- For `--resume <id>`, the owner is the account that has `projects/*/<id>.jsonl`. When more than one account has the session, the newest copy wins.
- For `-c`, the owner is the account with the newest `*.jsonl` in `projects/<encoded cwd>/`. When no account has a session for the current directory, ccx uses automatic selection.

Then:

1. If the owner is not limited, ccx launches the owner. It copies no files.
2. If the owner is limited (the same rules as automatic selection), ccx picks a target account with automatic selection. It copies the session with the handoff rules above and runs `claude --resume <id>` in the target. ccx changes `-c` to `--resume <id>`. If the handoff writes a fork, ccx resumes the fork.
3. If all accounts are limited, ccx launches the owner and shows a warning.
4. If the handoff refuses, for example because the session is open, ccx launches the owner and shows a warning.

On a terminal, ccx shows a banner that names the owner, the target and the reason, for example `ccx: handoff main → angel2 (main limited: 5h 92%)`. For a fork, the banner also shows the new session id. ccx shows the banner and the warnings only when stdout and stderr are terminals, and not for `-p`.

A bare `ccx --resume` (with no id, or with a search term) opens the session picker of Claude Code. ccx cannot know which session you will select. Thus ccx does no handoff. It picks the account with automatic selection and passes `--resume` with no change.

`ccx @name -c` and `CCX_ACCOUNT=name ccx -c` use only that account. They never do a handoff.

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

### Add a new account

```sh
ccx manage add work                            # Show the plan, then ask for confirmation.
ccx manage add work --email user@example.com   # Give the email to `claude auth login`.
ccx manage add work --dir ~/accounts/work      # Use a different directory.
ccx manage add work --no-login                 # Do not log in.
ccx manage add work -y                         # Do not ask. Use this in scripts.
```

`ccx manage add NAME` makes a new account in one step. The name must match `^[a-z0-9][a-z0-9_-]*$`. Add refuses a name or a directory that an account in the config uses. Add shows the plan and changes nothing until you confirm. When there is no terminal and you do not give `-y`, add stops and changes nothing.

After you confirm, add does these steps:

1. It makes the directory `~/.claude-NAME` (or `--dir`) with mode 0700.
2. It links the shared setup of the base account into the directory, as `ccx manage sync` does.
3. It runs `claude auth login` with `CLAUDE_CONFIG_DIR` set to the new directory. With `--email`, it runs `claude auth login --email <email>`. With `--no-login`, add does not do steps 3 and 4.
4. It runs `claude auth status --json` in the new account and compares the email with `--email`. If the emails are different, add shows a warning and asks before it writes the config. When there is no terminal, add stops and does not change the config.
5. It adds the account (the name and the directory) to `config.toml`. `config.toml` gets no email addresses.
6. It writes the email only to `config.local.toml`, with mode 0600.
7. It shows the kind, the tier and the weight that automatic selection uses, for example `work: kind max, tier Max 5x, weight 5`. An unknown tier shows the raw value.

Add keeps the other entries and the comments in the two files. It writes each file to a temporary file and then renames it. Add never reads or keeps the Claude Code credentials. Only `claude auth login` touches them.

If the login fails, the config does not change. Run `ccx manage add` again. Add accepts a directory that exists when it contains only Claude Code files. To add a directory that has a login already, use `ccx manage adopt`.

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
| `service` | The installed background service. Each path in the job (ccx and `claude`) exists. If not, doctor tells you to run `ccx manage service install` again. Doctor shows no line when no service is installed. |
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
- `reset` shows that the reset time of the window is in the past. The usage data is from the last window, so ccx does not know the current percent. In the JSON, `reset` is `true` and `percent` is `null`.
- `SOURCE` is `/usage` or `cachedUsageUtilization`. `AGE` is the age of the usage data.

ccx uses the usage cache when it is less than `usage.fresh_minutes` old (default 15). If not, ccx runs the `/usage` fetch for each account at the same time, and waits for a maximum of `usage.fetch_timeout_secs` (default 5). This time includes the read of the `claude` output. If the fetch fails, ccx uses `cachedUsageUtilization` from the `.claude.json` of the account. If the two sources are missing, ccx shows `unknown`. A failed fetch never makes the command fail.

The exit code is:

| Code | Meaning |
|---|---|
| 0 | At least one enabled account is logged in and not limited. |
| 3 | All logged-in enabled accounts are limited: session usage is `pick.session_avoid` (default 90) or more, or weekly usage is `pick.weekly_avoid` (default 95) or more. |
| 4 | No enabled account is logged in. |

`ccx status` has no session model. Thus, a scoped weekly window does not make an account limited here. Unknown usage and a window that shows `reset` do not make an account limited.

### Refresh the usage cache

```sh
ccx usage refresh angel2   # Fetch the usage of one account.
ccx usage refresh --all    # Fetch the usage of all enabled accounts.
```

The refresh writes `~/.local/state/ccx/usage/<name>.json`. ccx writes a temporary file at mode 0600, then renames it. Thus, a reader never sees a partial file. The refresh waits for a maximum of 30 seconds for each account. The exit code is 1 when a fetch fails.

### Automatic refresh

ccx keeps the usage cache fresh, and most launches do not wait for a fetch. This applies only to an automatic launch (no `@name` and no `CCX_ACCOUNT`).

1. When the usage data of each account is less than `usage.fresh_minutes` old (default 15), ccx uses it and does not wait. The usage data is the newer of the ccx cache and `cachedUsageUtilization`.
2. When the usage data of an account is older or missing, ccx first runs a blocking fetch for that account. The blocking fetch has a limit of `usage.fetch_timeout_secs` (default 5). This limit includes the time to wait for the refresh lock. If the fetch does not complete in time, ccx uses the old data or `cachedUsageUtilization` and continues the launch.
3. Before ccx starts `claude`, it starts `ccx usage refresh --all --background` as a detached process. This process continues after ccx starts `claude`. It writes nothing to the terminal.

All fetches use one file lock: `~/.local/state/ccx/refresh.lock`. ccx records the start time of the last fetch in `~/.local/state/ccx/refresh.json`.

| Fetch | When a different process holds the lock | The 60-second rule |
|---|---|---|
| Background refresh | It exits with no fetch. | It exits with no fetch when the last fetch started less than `usage.refresh_min_gap_secs` (default 60) ago. |
| Blocking fetch of a launch | It waits for the lock within its time limit. Then it reads the cache again. | It does not use the rule. |
| `ccx usage refresh` and `ccx status` | They wait for the lock. | They do not use the rule. |

ccx releases the lock before it starts the background refresh and before it starts `claude`. `ccx @name` fetches nothing and starts no refresh. Set `CCX_NO_REFRESH=1` to stop the blocking fetch and the background refresh of a launch.

### Background service

The background service is optional. It runs `ccx usage refresh --all --background` every 300 seconds. Thus, the usage cache is fresh before a launch needs it. Stale-while-revalidate works without the service.

```sh
ccx manage service install     # Write the job for this system and load it.
ccx manage service status      # Show the job, the last refresh and the age of each usage cache.
ccx manage service uninstall   # Stop the job and remove its files.
```

| System | What `install` does |
|---|---|
| macOS | It writes `~/Library/LaunchAgents/io.github.thetechchild.ccx.refresh.plist` with `RunAtLoad` and `StartInterval` 300, and loads it with `launchctl bootstrap`. |
| Linux with systemd | It writes `ccx-refresh.service` and `ccx-refresh.timer` in `~/.config/systemd/user/` and runs `systemctl --user enable --now ccx-refresh.timer`. When linger is off, it tells you to run `loginctl enable-linger`. Without linger, the timer runs only while you are logged in. |
| Linux with no systemd | It prints a crontab line. Add the line with `crontab -e`. ccx never edits the crontab and changes no files. |

- The job uses the absolute path of ccx and the absolute path of `claude`. ccx finds the `claude` path when you run `install`, and puts it in `CCX_CLAUDE_BIN`. The `PATH` of the job is the directory of `claude`, then `/usr/bin:/bin:/usr/sbin:/sbin` ([ADR 0007](docs/adr/0007-m0-manual-tests.md)).
- When `claude` moves, for example after a mise upgrade, the job cannot find it. `ccx manage service status` and `ccx manage doctor` then show an error. Run `ccx manage service install` again.
- The job is a background refresh. It uses the same lock and the same `usage.refresh_min_gap_secs` rule as the refresh of a launch. Thus, a job run and a launch never fetch at the same time.
- A second `install` with no changes does nothing. `status` works also when no service is installed. Its exit code is 1 when a path in the job does not exist.

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
weight = 5                 # Optional. Overrides the weight of the tier for automatic selection.

[pick]                     # Optional. The limits for automatic selection.
session_avoid = 90         # Session usage (%) that makes an account limited.
weekly_avoid = 95          # Weekly usage (%) that makes an account limited.
scoped_avoid = 98          # Matched scoped weekly usage (%) that makes an account limited.
stickiness = 10            # Points within the best score that keep the last account.

[usage]                    # Optional. The limits of the automatic refresh.
fresh_minutes = 15         # Usage data that is less old than this is fresh.
fetch_timeout_secs = 5     # The time limit of the blocking fetch of a launch.
refresh_min_gap_secs = 60  # The minimum time between two background refreshes.
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
| `CCX_SERVICE_KIND` | Input, for tests. Select the service manager of `ccx manage service`: `launchd`, `systemd` or `cron`. |
| `CCX_NO_REFRESH=1` | Input. An automatic launch does not fetch usage and does not start the background refresh. |

ccx removes `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN` and `CLAUDE_SECURESTORAGE_CONFIG_DIR` before it starts `claude`, because they override the account login. On a terminal, ccx shows one warning for each variable that it removes. When stdout or stderr is not a terminal, or for `-p` runs, ccx shows no banner and no warnings.

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

The completion files in `completions/` come from the CLI definition. A test fails when they do not agree with the CLI. To update them, run `CCX_UPDATE_GOLDEN=1 mise run test` and check the diff.

### Make a release

[cargo-dist](https://opensource.axo.dev/cargo-dist/) makes the release. The config is in `dist-workspace.toml`. The workflow is `.github/workflows/release.yml`. Do not edit the workflow by hand. Change `dist-workspace.toml` and run `mise exec -- dist generate`.

The release workflow does not use mise. It uses rustup on each runner, and rustup reads the Rust version from `rust-toolchain.toml`. When you change the Rust version in `mise.toml`, change it in `rust-toolchain.toml` too.

1. Make sure that `version` in `Cargo.toml` is the new version, and that CI is green on `main`.
2. Make sure that the plan is correct: `mise exec -- dist plan`.
3. Push a tag for the version, for example `git tag v0.1.0 && git push origin v0.1.0`.
4. The release workflow builds the archives for `aarch64-apple-darwin`, `x86_64-apple-darwin`, `aarch64-unknown-linux-gnu` and `x86_64-unknown-linux-gnu`, and makes the GitHub release with the archives and `sha256.sum`.

## Privacy rule

Do not put a real email address in this repository. Use `@example.com` addresses in code, tests and docs. The `check-emails` check fails on all other addresses, except addresses that contain `noreply`.

## License

MIT. Refer to [LICENSE](LICENSE).

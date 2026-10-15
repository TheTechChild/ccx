# ccx

ccx is a command-line tool. It starts Claude Code on the account that has the most usage available.

ccx is for a person who has more than one Claude Code account, for example a personal Max account and a work account. Each account has its own configuration directory. ccx reads the usage of each account and starts `claude` on the account that has the most capacity available. You can also name the account.

> **Status:** Version 0.1.2. `ccx @name`, automatic selection, `ccx status`, `ccx usage refresh`, `ccx manage adopt`, `ccx manage add`, `ccx manage bootstrap`, `ccx manage login`, `ccx manage logout`, `ccx manage edit`, `ccx manage edit-config`, `ccx manage rm`, `ccx manage list`, `ccx manage spend`, `ccx manage shell-init`, `ccx manage sync` (also `--prefs`), `ccx manage doctor`, `ccx manage service`, `ccx handoff`, the automatic handoff on `ccx -c` and `ccx --resume <id>`, and API-key accounts as a paid fallback are available. Refer to [PLAN.md](PLAN.md) for the project history.

## Credential policy

ccx never reads, stores or refreshes Claude Code credentials. ccx does not read `.credentials.json` or the keychain secrets of Claude Code. All logins go through `claude auth login`. ccx gets the usage data from the `claude` program. Refer to [ADR 0002](docs/adr/0002-no-credential-access.md).

The only secrets that ccx reads are the keys of [API-key accounts](#api-key-accounts-paid-fallback). ccx stores these keys itself, as the OS secret store items `ccx-NAME` or in key files at mode 0600. Refer to [ADR 0010](docs/adr/0010-api-key-store.md).

## Install

Install ccx with [mise](https://mise.jdx.dev):

```sh
mise use -g github:TheTechChild/ccx@latest
ccx --version
```

To get a new release later, run `mise upgrade github:TheTechChild/ccx`.

mise does not show a release that is less than 24 hours old (the mise setting `minimum_release_age`). Thus, for 24 hours after a release, mise installs the version before it. To get a new release immediately, do one of these steps:

- Exclude ccx from the setting one time for all installs: `mise settings add minimum_release_age_excludes "github:TheTechChild/ccx"`.
- Ignore the setting for one command: `mise use -g github:TheTechChild/ccx@latest --minimum-release-age 0s` or `mise upgrade github:TheTechChild/ccx --minimum-release-age 0s`.

mise downloads the release archive for your system from GitHub and puts `ccx` on `PATH`. The releases are for macOS and Linux, on x86_64 (amd64) and arm64 (aarch64). Each release has a `sha256.sum` file with the checksums of the archives.

You must also have Claude Code (`claude`) on `PATH`.

Each archive also has the static completion files in `completions/`: `_ccx` (zsh), `ccx.bash` and `ccx.fish`. These files do not complete the account names. To get the account names and the aliases, use `ccx manage shell-init` (refer to [Add completions and aliases to your shell](#add-completions-and-aliases-to-your-shell)).

## Quick start

1. Add the accounts that are on this machine already. These are the directories that you logged in to with `claude auth login`. The base account uses `~/.claude`. Each other account uses its own directory, for example `~/.claude-angel2`. Run `ccx manage adopt` to add them to the ccx config.
2. Add each new account: `ccx manage add NAME`. Add makes the directory `~/.claude-NAME`, links the shared setup of the base account, runs `claude auth login` and writes the config. Run `ccx manage add` with no NAME to start the wizard. Refer to [Add a new account](#add-a-new-account). For an Anthropic API key, refer to [Add an API-key account](#add-an-api-key-account).
3. Add completions and aliases to your shell profile: `eval "$(ccx manage shell-init zsh)"`. For example, `aliases = ["claude2"]` in account `angel2` gives `alias claude2='ccx @angel2'`.
4. Start Claude Code:
   - `ccx` selects the account that has the most capacity available.
   - `ccx @angel2` starts account `angel2`.
5. Show the usage of each account: `ccx status`.

The bare `claude` command stays on the base account. ccx never makes an alias for `claude`.

On a new machine that has a `config.toml` already, use `ccx manage bootstrap` for all the setup steps (refer to [Set up a new machine](#set-up-a-new-machine)).

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
6. Automatic selection never selects an [API-key account](#api-key-accounts-paid-fallback) while a subscription account is not limited. When all subscription accounts are limited, ccx asks before it uses a paid account.

Before ccx starts `claude`, it shows one banner line on stderr, for example `ccx → angel2 · 5h 23% · wk 61% (resets Thu 09:00) · /usage 4m`. ccx does not show the banner or the warnings when stdout or stderr is not a terminal, or for `-p` runs. ccx writes the selected account to `state.json` at mode 0600.

### API-key accounts (paid fallback)

An API-key account uses an Anthropic API key, not a subscription login (`kind = "api_key"` in `config.toml`). Each request costs money. Thus, ccx uses it only as a paid fallback:

```sh
ccx                         # All subscription accounts are limited: ccx asks first.
ccx --allow-paid            # Use the paid fallback with no prompt. Use this in scripts.
ccx --allow-paid @paid -p hi
ccx @paid --allow-paid -c   # --allow-paid can also come just after @name.
```

- When all subscription accounts are limited, ccx asks `All subscription accounts are limited. Use paid account NAME? [y/N]`. If you answer yes, ccx starts the paid account. If you answer no, ccx uses the soonest-reset rule (step 5 above).
- When there is no terminal, ccx never uses a paid account unless you give `--allow-paid`. It uses the soonest-reset rule.
- `ccx @name` on an API-key account also asks first. When there is no terminal and you do not give `--allow-paid`, ccx stops and does not start `claude`.
- `--allow-paid` must be the first argument, or come just after `@name`. In a different position, ccx sends it to `claude`.
- ccx sets `ANTHROPIC_API_KEY` only in the environment of the paid `claude` process. ccx removes `ANTHROPIC_API_KEY` from the environment of each subscription launch. The banner, the warnings and `CCX_DRY_RUN=1` never show the key. The dry run shows `ANTHROPIC_API_KEY=<hidden>`.
- If ccx cannot get the key, it shows an error and does not start `claude`.
- `ccx status` and `ccx manage list` show the kind `api-key (paid)` with no usage. ccx does not run the `/usage` fetch for an API-key account.
- `ccx handoff <id> @paid` copies the session, then asks before it resumes on the paid account.
- The base account `~/.claude` cannot be an API-key account.
- `ccx manage spend` shows an estimate of the money that each API-key account used in a month. Refer to [Show the spend of API-key accounts](#show-the-spend-of-api-key-accounts).

To add an API-key account, refer to [Add an API-key account](#add-an-api-key-account).

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
3. If the owner is an [API-key account](#api-key-accounts-paid-fallback) and a subscription account is not limited, ccx hands off to the subscription account that automatic selection picks. It does not ask.
4. If all subscription accounts are limited, ccx asks `All subscription accounts are limited. Use paid account NAME? [y/N]`. NAME is the owner when the owner is an API-key account, else the first API-key account. With `--allow-paid`, ccx does not ask. If you answer yes, ccx resumes on the paid account (with a handoff when the owner is a subscription account). If you answer no, or there is no terminal and no `--allow-paid`, the session stays in a subscription owner and ccx shows a warning. A session of an API-key owner goes to the subscription account with the soonest reset.
5. If the handoff refuses, for example because the session is open, ccx launches the owner and shows a warning.

On a terminal, ccx shows a banner that names the owner, the target and the reason, for example `ccx: handoff main → angel2 (main limited: 5h 92%)`. For a fork, the banner also shows the new session id. ccx shows the banner and the warnings only when stdout and stderr are terminals, and not for `-p`.

A bare `ccx --resume` (with no id, or with a search term) opens the session picker of Claude Code. ccx cannot know which session you will select. Thus ccx does no handoff. It picks the account with automatic selection and passes `--resume` with no change.

`ccx @name -c` and `CCX_ACCOUNT=name ccx -c` use only that account. They never do a handoff.

## Manage accounts

### Set up a new machine

```sh
ccx manage bootstrap               # Ask before it installs the service.
ccx manage bootstrap --service     # Install the service without a prompt.
ccx manage bootstrap --no-service  # Do not install the service.
```

`ccx manage bootstrap` sets up ccx on a new macOS or Linux machine, also on an SSH server. It uses `config.toml` (`~/.config/ccx/config.toml`, for example a link to the file in your config repo, or `$CCX_CONFIG`). It does these steps:

1. It finds `claude`. If claude is not installed, it shows the installer command, `curl -fsSL https://claude.ai/install.sh | bash`, and stops with exit code 1.
2. It gets the base account `~/.claude` from `[base].repo`. If `~/.claude` does not exist, it clones the repo. If `~/.claude` exists and is not a git repo, it runs `git init`, `git remote add`, `git fetch` and `git checkout`. Before the checkout, it stops if a file of the repo is in `~/.claude` already. It never overwrites or deletes a file that is in `~/.claude`.
3. It makes each missing account directory (mode 0700) and runs `ccx manage sync --all`.
4. It logs in to each account that is not logged in, as `ccx manage login --all` does, with the email check.
5. It offers to install the service (`ccx manage service install`). When there is no terminal, it does not install the service unless you give `--service`.
6. It shows `ccx status` and the line to add to your shell profile, for example `eval "$(ccx manage shell-init zsh)"`.

The emails for the email check come only from `~/.config/ccx/config.local.toml`. You never commit this file, so it is possibly not on a new machine. If it is missing, bootstrap shows a warning and does not check the emails. To check the emails, add the file before you run bootstrap.

When there is no browser (an SSH session, or Linux with no `DISPLAY` and no `WAYLAND_DISPLAY`), `claude auth login` prints a login URL. ccx shows a note before the login. Open the URL on a different device and log in. Then paste the code into the claude prompt. ccx does not capture or change the output of claude, and it adds no flag to `claude auth login`. `ccx manage login` and `ccx manage add` show the same note.

You can run bootstrap again. A step that is done already shows OK and changes nothing. At the end, bootstrap shows a summary with one line for each step: OK, DONE, SKIP or FAIL. When a step fails, bootstrap continues with the next steps and then exits with code 1.

### Adopt the accounts on this machine

```sh
ccx manage adopt                                      # On a terminal: start the wizard.
ccx manage adopt -y                                   # Do not ask. Use this in scripts.
ccx manage adopt --rename angel2=work --skip test -y  # Change a name, skip a directory.
```

On a terminal with no `-y`, `--rename` or `--skip`, adopt starts a wizard. For each directory that is not in the config, you select one of these: adopt it with its name, adopt it with a different name, or skip it. Then adopt shows the summary. `--rename OLD=NEW` and `--skip NAME` give the same choices in scripts. OLD and NAME are the names that adopt found, for example `angel2` for `~/.claude-angel2`. You can give each flag more than one time.

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
ccx manage add                                 # On a terminal: start the wizard.
ccx manage add work                            # Show the plan, then ask for confirmation.
ccx manage add work --email user@example.com   # Give the email to `claude auth login`.
ccx manage add work --dir ~/accounts/work      # Use a different directory.
ccx manage add work --no-login                 # Do not log in.
ccx manage add work --no-prefs                 # Do not copy the MCP servers and the preferences.
ccx manage add work -y                         # Do not ask. Use this in scripts.
```

With no NAME on a terminal, add starts a wizard. The wizard asks for the name, then the directory (the default is `~/.claude-NAME`). It shows the items that sync links. Then it asks if you want to log in now, for the email, and if you want to copy the MCP servers and the preferences. Then add shows the plan and asks for confirmation. At the end, add shows a summary with the email and the tier. Each wizard value has a flag. With no NAME and no terminal, add fails and shows the flags. Push Esc or Ctrl-C to stop the wizard. The wizard changes nothing before the last confirmation.

`ccx manage add NAME` makes a new account in one step. The name must match `^[a-z0-9][a-z0-9_-]*$`. Add refuses a name or a directory that an account in the config uses. Add shows the plan and changes nothing until you confirm. When there is no terminal and you do not give `-y`, add stops and changes nothing.

After you confirm, add does these steps:

1. It makes the directory `~/.claude-NAME` (or `--dir`) with mode 0700.
2. It links the shared setup of the base account into the directory, as `ccx manage sync` does.
3. It runs `claude auth login` with `CLAUDE_CONFIG_DIR` set to the new directory. With `--email`, it runs `claude auth login --email <email>`. With `--no-login`, add does not do steps 3 and 4.
4. It runs `claude auth status --json` in the new account and compares the email with `--email`. If the emails are different, add shows a warning and asks before it writes the config. When there is no terminal, add stops and does not change the config.
5. It adds the account (the name and the directory) to `config.toml`. `config.toml` gets no email addresses.
6. It writes the email only to `config.local.toml`, with mode 0600.
7. It shows a summary with the directory and the email. Then it shows the kind, the tier and the weight that automatic selection uses, for example `work: kind max, tier Max 5x, weight 5`. An unknown tier shows the raw value.
8. It copies the user MCP servers and the allowlisted preferences of the base account into the new account, as `ccx manage sync --prefs` does. With `--no-prefs` or `--no-login`, add does not do this step. If this step fails, add shows a warning. Run `ccx manage sync --prefs NAME` to try again.

Add keeps the other entries and the comments in the two files. It writes each file to a temporary file and then renames it. Add never reads or keeps the Claude Code credentials. Only `claude auth login` touches them.

If the login fails, the config does not change. Run `ccx manage add` again. Add accepts a directory that exists when it contains only Claude Code files. To add a directory that has a login already, use `ccx manage adopt`.

### Add an API-key account

```sh
ccx manage add paid --api-key                          # Read the key with a hidden prompt. Keep it in the OS secret store.
ccx manage add paid --api-key --key-command 'pass show anthropic'
ccx manage add paid --api-key --key-file               # Keep the key in ~/.config/ccx/keys/paid (mode 0600).
op read 'op://vault/anthropic/key' | ccx manage add paid --api-key -y   # Read the key from stdin.
```

`ccx manage add NAME --api-key` makes an [API-key account](#api-key-accounts-paid-fallback). It shows the plan and asks, as for a subscription account. Then it does these steps:

1. It reads the key with a hidden prompt. When stdin is not a terminal, it reads one line of stdin.
2. It stores the key in the OS secret store, as the item `ccx-NAME`. On macOS, the store is the login keychain (through `security`). On Linux, the store is the Secret Service (through `secret-tool` from libsecret). The key goes to these commands only on stdin.
3. It makes the directory and runs sync, as for a subscription account. There is no login and no copy of the preferences.
4. It writes the account to `config.toml` with `kind = "api_key"` and `key_store = "keyring"`. `config.toml` never gets the key.

With `--key-command COMMAND`, ccx keeps no copy of the key. ccx runs the command with `sh -c` at each paid launch and reads the key from its stdout, for example from a password manager. Add runs the command one time to make sure that it gives a key. If the command fails or gives no output, ccx shows an error and does not start `claude`. The account entry gets `key_command = "COMMAND"`.

On a server with no Secret Service (no D-Bus session, or no unlocked keyring), the OS secret store is not available. Then add shows the error and two fallbacks:

1. `key_command`: run `ccx manage add NAME --api-key --key-command '<command>'`.
2. A key file: on a terminal, add asks `Write the key to ~/.config/ccx/keys/NAME (mode 0600)?`. With no terminal, run `ccx manage add NAME --api-key --key-file`.

ccx writes the key file only after you confirm. The file has mode 0600, in a directory with mode 0700. The account entry gets `key_store = "file"`. ccx refuses to read a key file that has a different mode. To fix the mode, run `chmod 600 ~/.config/ccx/keys/NAME`.

`ccx manage rm NAME --purge` deletes the item `ccx-NAME` or the key file of an API-key account. Without `--purge`, rm keeps the key and shows the command that deletes it. `ccx manage login` and `logout` refuse an API-key account, and `ccx manage login --all` skips it.

### Log in and log out

```sh
ccx manage login work      # Log in to one account.
ccx manage login --all     # Log in to each account that is not logged in.
ccx manage logout work     # Ask, then log out of one account.
ccx manage logout work -y  # Do not ask. Use this in scripts.
```

`ccx manage login NAME` runs `claude auth login --email <email>` with `CLAUDE_CONFIG_DIR` set to the directory of the account. For the base account, `CLAUDE_CONFIG_DIR` is not set. The email comes only from `config.local.toml`. After the login, ccx runs `claude auth status --json` and compares the two emails. If the emails are different, the command fails and tells you how to log out. If `config.local.toml` has no email for the account, ccx shows a warning, runs `claude auth login` with no email and does not compare.

`ccx manage login --all` runs `claude auth status` for each account. It logs in only to the accounts that are not logged in. Then it shows one summary line for each account. The exit code is 1 when a login failed.

`ccx manage logout NAME` runs `claude auth logout` for that account only.

ccx never reads, keeps or refreshes the Claude Code credentials. Only `claude auth login` and `claude auth logout` touch them.

### Change an account

```sh
ccx manage edit                              # On a terminal: start the wizard.
ccx manage edit work --weight 5              # Change the weight for automatic selection.
ccx manage edit work --disable               # Do not use the account for automatic selection.
ccx manage edit work --enable
ccx manage edit work --email user@example.com # Change the expected email.
ccx manage edit-config                       # Open config.toml in $EDITOR.
```

With no NAME on a terminal, edit starts a wizard. You select the account. The wizard shows the current values and asks for the email, the weight and if automatic selection uses the account. Then it shows a summary and asks before it writes.

`ccx manage edit NAME` changes only the fields that you give. It keeps the other entries, the order and the comments. It validates the two files before it writes them. It refuses a value that is not valid, for example a weight of 0 or less. `--email` writes the email only to `config.local.toml` (mode 0600). `config.toml` never gets an email.

`ccx manage edit-config` opens a copy of `config.toml` in `$EDITOR` (else `$VISUAL`, else `vi`). After the editor exits, ccx validates the copy. If the copy is valid, ccx writes it to `config.toml`. If not, `config.toml` does not change, and ccx tells you where the copy is.

### Remove an account

```sh
ccx manage rm                     # On a terminal: start the wizard.
ccx manage rm work                # Remove the account from the config. Keep the directory.
ccx manage rm work --purge        # Also log out and move the directory to the trash.
ccx manage rm work --purge -y     # Do not ask. Use this in scripts.
```

`ccx manage rm NAME` shows the plan and asks before it changes files. When there is no terminal and you do not give `-y`, rm stops and changes nothing. Rm removes the account from `config.toml` and from `config.local.toml`. It keeps the directory. It shows the name of the macOS keychain item of the account, for example `Claude Code-credentials-1a2b3c4d`. ccx does not remove the keychain item. To remove it, run the `security delete-generic-password` command that rm shows.

With `--purge`, rm also does these steps:

1. It runs `claude auth logout` for the account. If the logout fails, rm shows a warning and continues.
2. It deletes the credentials file in the directory if the file is there. It never reads the file.
3. It moves the directory to `~/.local/state/ccx/trash/NAME-<time>`. The trash directory has mode 0700. ccx never deletes the directory. To delete it, remove it from the trash yourself.

Rm refuses the base account `~/.claude`.

With no NAME on a terminal, rm starts a wizard. You select the account and select if you want the purge. Then rm shows the plan. Before a purge, you must type the account name to confirm.

### List the accounts

```sh
ccx manage list          # Show a table: name, directory, email, aliases and kind.
ccx manage list --json   # Show stable JSON for scripts.
```

### Show the spend of API-key accounts

```sh
ccx manage spend                       # The current month, all API-key accounts.
ccx manage spend --month 2026-09       # A different month (local time).
ccx manage spend @paid                 # Only one account.
ccx manage spend --month 2026-09 --json  # Show stable JSON for scripts.
```

The report shows one row for each account and model, a total for each account and a grand total. It shows only accounts with `kind = "api_key"`. Subscription accounts are not in the report.

How ccx calculates the spend:

1. ccx reads the session transcripts (`projects/**/*.jsonl`) in the directory of each API-key account.
2. For each assistant message in the month, ccx adds the `usage` token counts for its model: input, output, cache write and cache read. ccx counts each message one time, also when the transcript has more than one line for it.
3. ccx multiplies the token counts by the price of the model. The built-in price table has the current Claude models. A `[pricing.<model>]` entry in `config.toml` replaces the built-in price of that model (refer to [Configuration](#configuration)). A model name with a date suffix, for example `claude-sonnet-4-5-20250929`, also uses the price of `claude-sonnet-4-5`.

Important:

- The report is an estimate from local files. It is not your bill. ccx does not call a billing API and does not read the API key for the report.
- A model with no price shows as `unpriced` with its token counts. The totals do not include it. Add a `[pricing.<model>]` entry to give it a price.
- ccx uses the cache write price for all cache writes. It does not use a different price for the 1-hour cache or for long prompts.
- When a transcript line is not valid JSON, or an assistant message has no timestamp, model or usage, ccx skips the line. The report shows the number of skipped lines.
- `ccx handoff` copies a session with its earlier messages. When you hand off a session to an API-key account, the report also counts the copied messages that are in the month.

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
ccx manage sync --all -y           # Ask no questions.
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

### Copy MCP servers and preferences

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

### Check the setup with doctor

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
- `KIND` is `subscriptionType` from `claude auth status`. For an API-key account, it is `api-key (paid)`, and the usage columns show `-`. `TIER` comes from `oauthAccount.organizationRateLimitTier` in the `.claude.json` of the account. If ccx does not know the tier, it shows the raw value.
- `SESSION`, `WEEKLY` and `SCOPED` show the percent used. The time in parentheses is the time until the window resets.
- `reset` shows that the reset time of the window is in the past. The usage data is from the last window, so ccx does not know the current percent. In the JSON, `reset` is `true` and `percent` is `null`.
- `SOURCE` is `/usage` or `cachedUsageUtilization`. `AGE` is the age of the usage data.

ccx uses the usage cache when it is less than `usage.fresh_minutes` old (default 15). If not, ccx runs the `/usage` fetch for each account at the same time, and waits for a maximum of `usage.fetch_timeout_secs` (default 5). This time includes the read of the `claude` output. If the fetch fails, ccx uses `cachedUsageUtilization` from the `.claude.json` of the account. If the two sources are missing, ccx shows `unknown`. A failed fetch never makes the command fail.

The exit code is:

| Code | Meaning |
|---|---|
| 0 | At least one enabled account is logged in and not limited. API-key accounts do not count for codes 0, 3 and 4. |
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

The background service is optional. It runs `ccx usage refresh --all --background --scheduled` every 60 seconds. Each run fetches only the accounts that are due (refer to [Adaptive interval](#adaptive-interval)). Thus, the usage cache is fresh before a launch needs it. Stale-while-revalidate works without the service.

```sh
ccx manage service install     # Write the job for this system and load it.
ccx manage service status      # Show the job, the last refresh, and the cache age and the next fetch of each account.
ccx manage service uninstall   # Stop the job and remove its files.
```

| System | What `install` does |
|---|---|
| macOS | It writes `~/Library/LaunchAgents/io.github.thetechchild.ccx.refresh.plist` with `RunAtLoad` and `StartInterval` 60, and loads it with `launchctl bootstrap`. |
| Linux with systemd | It writes `ccx-refresh.service` and `ccx-refresh.timer` in `~/.config/systemd/user/` and runs `systemctl --user enable --now ccx-refresh.timer`. When linger is off, it tells you to run `loginctl enable-linger`. Without linger, the timer runs only while you are logged in. |
| Linux with no systemd | It prints a crontab line. Add the line with `crontab -e`. ccx never edits the crontab and changes no files. |

- The job uses the absolute path of ccx and the absolute path of `claude`. ccx finds the `claude` path when you run `install`, and puts it in `CCX_CLAUDE_BIN`. The `PATH` of the job is the directory of `claude`, then `/usr/bin:/bin:/usr/sbin:/sbin` ([ADR 0007](docs/adr/0007-m0-manual-tests.md)).
- When `claude` moves, for example after a mise upgrade, the job cannot find it. `ccx manage service status` and `ccx manage doctor` then show an error. Run `ccx manage service install` again.
- The job is a background refresh. It uses the same lock and the same `usage.refresh_min_gap_secs` rule as the refresh of a launch. Thus, a job run and a launch never fetch at the same time.
- A second `install` with no changes does nothing. `status` works also when no service is installed. Its exit code is 1 when a path in the job does not exist.
- The crontab line runs each minute.

#### Adaptive interval

Each run of the job reads the usage cache. For each account, it calculates an interval. The account is due when its cache is older than its interval. The job fetches only the due accounts. When no account is due, the job starts no `claude` process. These rules set the interval:

| Data | Effect on the interval |
|---|---|
| The highest percent in `limits[]` | At 0%, the interval is `usage.max_interval_secs`. At 90% or more, it is `usage.min_interval_secs`. Between these values, the interval is proportional. |
| The change since the fetch before | When the percent increases, the interval is at most half of the time until the account gets to 90% at that rate. |
| The soonest `resets_at` | The interval stops at the reset of a window that is not at 0%. When a window has reset, the interval is `usage.min_interval_secs`. |
| The last ccx launch on the account | When the last launch is less than one hour ago, the interval is half. |

The interval is always between `usage.min_interval_secs` (default 120) and `usage.max_interval_secs` (default 1800). When a fetch fails, the job tries again after `usage.min_interval_secs`. Set `usage.adaptive = false` to fetch each account every 300 seconds.

The job writes the next due time of each account to `~/.local/state/ccx/schedule.json`. Each launch writes its time to `~/.local/state/ccx/launches/<name>`. `ccx manage service status` shows the next fetch of each account, for example `main    7m old, next fetch: 14:25 (in 3m, every 10m)`.

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
[launch]                   # Optional.
claude_bin = "~/.local/bin/claude"   # Optional. The path of claude. The default is the claude on PATH.

[base]                     # Optional. Used by `ccx manage bootstrap`.
repo = "https://github.com/example/claude-config.git"   # The git repo of the base account ~/.claude.

[accounts.main]
dir = "~/.claude"          # The base account. Exactly one account must use this directory.

[accounts.angel2]
dir = "~/.claude-angel2"
aliases = ["claude2"]      # Optional. `ccx manage shell-init` makes `alias claude2='ccx @angel2'`.
own = ["statusline.py"]    # Optional. Items that `ccx manage sync` does not link.
weight = 5                 # Optional. Overrides the weight of the tier for automatic selection.
enabled = true             # Optional. false: automatic selection does not use the account.

[accounts.paid]            # Optional. An API-key account: a paid fallback.
dir = "~/.claude-paid"
kind = "api_key"           # "subscription" (the default) or "api_key".
key_store = "keyring"      # "keyring" (the OS secret store item `ccx-paid`) or "file" (~/.config/ccx/keys/paid).
# key_command = "pass show anthropic"  # Or: a command that prints the key. Use key_store or key_command, not both.

[pick]                     # Optional. The limits for automatic selection.
session_avoid = 90         # Session usage (%) that makes an account limited.
weekly_avoid = 95          # Weekly usage (%) that makes an account limited.
scoped_avoid = 98          # Matched scoped weekly usage (%) that makes an account limited.
stickiness = 10            # Points within the best score that keep the last account.

[usage]                    # Optional. The limits of the automatic refresh.
fresh_minutes = 15         # Usage data that is less old than this is fresh.
fetch_timeout_secs = 5     # The time limit of the blocking fetch of a launch.
refresh_min_gap_secs = 60  # The minimum time between two background refreshes.
adaptive = true            # Adapt the interval of the service to the usage. false: every 300 seconds.
min_interval_secs = 120    # The shortest interval of the service. It must be 1 or more.
max_interval_secs = 1800   # The longest interval of the service. It must be min_interval_secs or more.

[prefs]                    # Optional. Changes to the allowlist of `ccx manage sync --prefs`.
allow = []                 # Keys to copy also.
deny = []                  # Keys not to copy.

[sync]                     # Optional. Top-level entries of ~/.claude that are not built-in shared items.
share = []                 # Entries that `ccx manage sync` links like a built-in shared item.
ignore = []                # Entries that sync does not share and does not show as UNKNOWN.

[pricing.claude-opus-5-5]  # Optional. Replaces the built-in price of a model for `ccx manage spend`.
input = 4                  # USD for each million input tokens.
output = 20                # USD for each million output tokens.
cache_write = 5            # USD for each million cache write tokens.
cache_read = 0.2           # USD for each million cache read tokens.
```

A `[pricing.<model>]` entry must have all four prices. Each price is 0 or more.

A name in `[sync] share` or `[sync] ignore` is a top-level entry of `~/.claude`. It must not be empty, `.` or `..`. It must not contain `/` or start with `.`. It must not be on the never-share list, be a built-in shared item, or be `skills`. A name cannot be in the two lists. A `share` name that is not in `~/.claude` has no effect.

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

ccx removes `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN` and `CLAUDE_SECURESTORAGE_CONFIG_DIR` before it starts `claude`, because they override the account login. For a paid launch on an API-key account, ccx then sets `ANTHROPIC_API_KEY` to the key of that account. On a terminal, ccx shows one warning for each variable that it removes. When stdout or stderr is not a terminal, or for `-p` runs, ccx shows no banner and no warnings.

If ccx cannot start `claude`, it exits with code 127.

## How ccx works

- Each account has its own Claude Code configuration directory. The base account uses `~/.claude`.
- ccx sets `CLAUDE_CONFIG_DIR` for the account that it selects, and then starts `claude`.
- ccx does not read, keep or refresh Claude Code credentials. All logins go through `claude auth login`.

## Development

[CONTRIBUTING.md](CONTRIBUTING.md) tells you how to set up the project, run the checks and propose a change. [AGENTS.md](AGENTS.md) has the module map, the test method and the hard rules. `CLAUDE.md` imports `AGENTS.md` for Claude Code.

This project uses [mise](https://mise.jdx.dev) for all tools.

1. Install the tools: `mise install`.
2. Run all checks: `mise run ci`.

The checks are `fmt`, `lint`, `test` and `check-emails`. You can run each check with `mise run <name>`.

The completion files in `completions/` come from the CLI definition. A test fails when they do not agree with the CLI. To update them, run `CCX_UPDATE_GOLDEN=1 mise run test` and check the diff.

### Make a release

[cargo-dist](https://opensource.axo.dev/cargo-dist/) makes the release. The config is in `dist-workspace.toml`. The workflow is `.github/workflows/release.yml`. Do not edit the workflow by hand. Change `dist-workspace.toml` and run `mise exec -- dist generate`.

The release workflow does not use mise. It uses rustup on each runner, and rustup reads the Rust version from `rust-toolchain.toml`. When you change the Rust version in `mise.toml`, change it in `rust-toolchain.toml` too.

In these steps, `vX.Y.Z` is the new version, for example `v0.1.3`.

1. Make a pull request that sets `version = "X.Y.Z"` in `Cargo.toml`, updates `Cargo.lock` (`mise exec -- cargo check`) and sets the version in the status line of this README. Use the commit message `chore(release): prepare vX.Y.Z`. Merge it when CI is green.
2. Update your local `main`: `git switch main && git pull`.
3. Make sure that the plan is correct: `mise exec -- dist plan`.
4. Push a tag for the version: `git tag vX.Y.Z && git push origin vX.Y.Z`.
5. The release workflow builds the archives for `aarch64-apple-darwin`, `x86_64-apple-darwin`, `aarch64-unknown-linux-gnu` and `x86_64-unknown-linux-gnu`, and makes the GitHub release with the archives and `sha256.sum`.

## Privacy rule

Do not put a real email address in this repository. Use `@example.com` addresses in code, tests and docs. The `check-emails` check fails on all other addresses, except addresses that contain `noreply`.

## License

MIT. Refer to [LICENSE](LICENSE).

# Manage accounts

The `ccx manage` commands set up a new machine, and add, log in, change, remove and list accounts.

```sh
ccx manage adopt         # Add the accounts that are on this machine.
ccx manage add work      # Make a new account and log in.
ccx manage login --all   # Log in to each account that is not logged in.
ccx manage edit work     # Change an account.
ccx manage rm work       # Remove an account from the config.
ccx manage list          # Show the accounts.
```

On a terminal, `add`, `edit` and `rm` with no NAME start a wizard. `adopt`, `add`, `rm` and `logout` take `-y`: they do not ask. With no terminal and no `-y`, `adopt`, `add` and `rm` stop and change nothing. ccx never reads, keeps or refreshes Claude Code credentials. Only `claude auth login` and `claude auth logout` touch them ([credential policy](../README.md#credential-policy)).

## Set up a new machine

```sh
ccx manage bootstrap               # Ask before it installs the service.
ccx manage bootstrap --service     # Install the service. Do not ask.
ccx manage bootstrap --no-service  # Do not install the service.
```

`ccx manage bootstrap` sets up a new macOS or Linux machine or SSH server. It uses [`config.toml`](configuration.md) (`~/.config/ccx/config.toml`, for example a link into your config repo, or `$CCX_CONFIG`).

1. Find `claude`. If it is missing, show `curl -fsSL https://claude.ai/install.sh | bash`, and stop with exit code 1.
2. Get the base account `~/.claude` from `[base].repo`. If `~/.claude` is missing, clone the repo. If it is not a git repo, run `git init`, `git remote add`, `git fetch` and `git checkout`. Stop before the checkout if a file of the repo is in `~/.claude` already. Never overwrite or delete a file in `~/.claude`.
3. Make each missing account directory (mode 0700) and run `ccx manage sync --all`.
4. Log in to each account that is not logged in (`ccx manage login --all`), with the email check.
5. Offer to install the service (`ccx manage service install`). With no terminal, install it only with `--service`.
6. Show `ccx status` and the line for your shell profile, for example `eval "$(ccx manage shell-init zsh)"`.

The email check uses only `~/.config/ccx/config.local.toml`, which you never commit. Without it, bootstrap shows a warning and skips the check. Add the file first.

You can run bootstrap again. A done step shows OK and changes nothing. At the end, bootstrap shows one line for each step: OK, DONE, SKIP or FAIL. When a step fails, bootstrap continues, then exits with code 1.

## Adopt the accounts on this machine

```sh
ccx manage adopt                                      # Start the wizard.
ccx manage adopt --rename angel2=work --skip test -y  # Rename, skip.
```

`ccx manage adopt` finds `~/.claude` (the base account, name `main`) and each `~/.claude-<name>` directory. It gets each email with `claude auth status --json`, with `CLAUDE_CONFIG_DIR` set to the directory (not for the base account).

On a terminal with no `-y`, `--rename` or `--skip`, adopt starts a wizard. For each directory not in the config, you adopt it, adopt it with a different name, or skip it. In scripts, `--rename OLD=NEW` and `--skip NAME` give the same choices. OLD and NAME are found names, for example `angel2` for `~/.claude-angel2`. Each flag can repeat.

Adopt shows the plan and writes nothing until you confirm. Then it:

1. Adds each new account to `config.toml` with its `dir`, and no email.
2. Writes the emails only to `config.local.toml`, with mode 0600.
3. Moves the stray files `~/.claude/.claude.json` and `~/.claude/.claude.json.lock` to `$XDG_STATE_HOME/ccx/backup/adopt-<time>` (or `~/.local/state/ccx/backup/adopt-<time>`) and shows the path. Adopt never deletes these files.

Adopt keeps the other entries and the comments of existing files. It writes a temporary file and renames it. When `config.toml` is a symlink, adopt changes the target and keeps the symlink. It writes `config.local.toml` at the overlay path, never next to the target. You can run adopt again. It adds only the missing accounts and emails.

## Add a new account

```sh
ccx manage add
ccx manage add work                            # Show the plan, then ask.
ccx manage add work --email user@example.com   # Email for `claude auth login`.
ccx manage add work --dir ~/accounts/work      # Use a different directory.
ccx manage add work --no-login                 # Do not log in.
ccx manage add work --no-prefs                 # Do not copy MCP servers and preferences.
```

`ccx manage add NAME` makes an account in one step. For an API key, refer to [Add an API-key account](api-key-accounts.md#add-an-api-key-account). The name must match `^[a-z0-9][a-z0-9_-]*$`. Add refuses a name or a directory that the config uses. It accepts an existing directory that contains only Claude Code files. For a directory with a login, use `ccx manage adopt`.

The wizard asks for the name and the directory (default `~/.claude-NAME`). It shows the items that sync links. Then it asks: log in now, the email, and copy the MCP servers and the preferences. Each value has a flag. Esc or Ctrl-C stops the wizard, which changes nothing before the last confirmation. With no NAME and no terminal, add fails and shows the flags.

After you confirm the plan, add:

1. Makes `~/.claude-NAME` (or `--dir`) with mode 0700.
2. Links the shared setup of the base account into it, as [`ccx manage sync`](sync.md) does.
3. Runs `claude auth login` (with `--email <email>` when you give it) with `CLAUDE_CONFIG_DIR` set to the new directory.
4. Compares the email from `claude auth status --json` with `--email`. If they differ, add shows a warning and asks before it writes the config. With no terminal, add stops and does not change the config.
5. Adds the name and the directory to `config.toml`, with no email.
6. Writes the email only to `config.local.toml`, with mode 0600.
7. Shows the directory, the email, and the kind, tier and weight for automatic selection, for example `work: kind max, tier Max 5x, weight 5`. An unknown tier shows the raw value.
8. Copies the user MCP servers and the allowlisted preferences of the base account, as [`ccx manage sync --prefs`](sync.md#copy-mcp-servers-and-preferences) does. On failure, add shows a warning. Run `ccx manage sync --prefs NAME` to try again.

`--no-login` skips steps 3, 4 and 8. `--no-prefs` skips step 8. Add keeps the other entries and the comments, and writes through a temporary file that it renames. If the login fails, the config does not change. Run add again.

## Log in and log out

```sh
ccx manage login work      # Log in to one account.
ccx manage login --all     # Log in to each logged-out account.
ccx manage logout work     # Ask, then log out.
ccx manage logout work -y  # Do not ask.
```

`ccx manage login NAME` runs `claude auth login --email <email>` with `CLAUDE_CONFIG_DIR` set to the directory of the account (not for the base account). The email comes only from `config.local.toml`. Then ccx compares it with `claude auth status --json`. If they differ, the command fails and tells you how to log out. With no email there, ccx shows a warning, logs in with no email and does not compare.

`--all` runs `claude auth status` for each account, logs in to each logged-out account, and shows one line for each. The exit code is 1 when a login failed.

`logout NAME` runs `claude auth logout` for one account.

### Log in with no browser

With no browser (an SSH session, or Linux with no `DISPLAY` and no `WAYLAND_DISPLAY`), `claude auth login` prints a login URL. `login`, `add` and `bootstrap` show a note first. Open the URL on a different device, log in, and paste the code into the claude prompt. ccx does not capture or change the output, and adds no flag.

## Change an account

```sh
ccx manage edit
ccx manage edit work --weight 5               # Weight for automatic selection.
ccx manage edit work --disable                # Exclude from automatic selection.
ccx manage edit work --enable
ccx manage edit work --email user@example.com # Change the expected email.
ccx manage edit-config                        # Open config.toml in $EDITOR.
```

The wizard asks for the account, then shows and asks for the email, the weight and if automatic selection uses it. It shows a summary and asks before it writes.

`ccx manage edit NAME` changes only the fields that you give. It keeps the other entries, the order and the comments. It validates the two files before it writes, and refuses a value that is not valid, for example a weight of 0 or less. `--email` writes the email only to `config.local.toml` (mode 0600).

`ccx manage edit-config` opens a copy of `config.toml` in `$EDITOR` (else `$VISUAL`, else `vi`). Then ccx writes it to `config.toml` if it is valid. If not, `config.toml` does not change, and ccx tells you where the copy is.

## Remove an account

```sh
ccx manage rm
ccx manage rm work                # Remove from the config. Keep the directory.
ccx manage rm work --purge        # Also log out and trash the directory.
```

`ccx manage rm NAME` shows the plan and asks first. It removes the account from `config.toml` and `config.local.toml`. It does not remove the macOS keychain item, for example `Claude Code-credentials-1a2b3c4d`. It shows the item and the `security delete-generic-password` command for it. Rm refuses the base account `~/.claude`.

With `--purge`, rm also:

1. Runs `claude auth logout` for the account. On failure, rm shows a warning and continues.
2. Deletes the credentials file in the directory, if it is there, but never reads it.
3. Moves the directory to `~/.local/state/ccx/trash/NAME-<time>` (mode 0700). ccx never deletes it. You delete it from the trash.

The wizard asks for the account and if you want the purge. Then rm shows the plan. To confirm a purge, you type the account name.

## List the accounts

```sh
ccx manage list          # Table: name, directory, email, aliases, kind.
ccx manage list --json   # Stable JSON for scripts.
```

# Manage accounts

## Set up a new machine

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

## Adopt the accounts on this machine

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

## Add a new account

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

## Log in and log out

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

## Change an account

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

## Remove an account

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

## List the accounts

```sh
ccx manage list          # Show a table: name, directory, email, aliases and kind.
ccx manage list --json   # Show stable JSON for scripts.
```

# API-key accounts (paid fallback)

An API-key account uses an Anthropic API key, not a subscription login (`kind = "api_key"` in `config.toml`). Each request costs money, so ccx uses it only as a paid fallback.

```sh
ccx
ccx --allow-paid          # No prompt. Use this in scripts.
ccx --allow-paid @paid -p hi
ccx @paid --allow-paid -c
```

## Paid launch rules

- When all subscription accounts are limited, ccx asks `All subscription accounts are limited. Use paid account NAME? [y/N]`. Yes starts the paid account. No uses the soonest-reset rule (step 5 of [Automatic selection](launch.md#automatic-selection)). With no terminal, ccx uses this rule unless you give `--allow-paid`.
- `ccx @name` on an API-key account also asks first. With no terminal and no `--allow-paid`, ccx stops.
- `--allow-paid` must be the first argument, or come just after `@name`. Elsewhere, ccx sends it to `claude`.
- ccx sets `ANTHROPIC_API_KEY` only for the paid `claude` process, and removes it from each subscription launch. The banner, the warnings and `CCX_DRY_RUN=1` never show the key (the dry run shows `ANTHROPIC_API_KEY=<hidden>`).
- If ccx cannot get the key, it shows an error and does not start `claude`.
- The base account `~/.claude` cannot be an API-key account.
- `ccx status` and `ccx manage list` show the kind `api-key (paid)` with no usage, and do not run the `/usage` fetch.
- `ccx handoff <id> @paid` copies the session, then asks before it resumes there.
- `ccx manage login` and `logout` refuse the account; `login --all` skips it.
- `ccx manage rm NAME --purge` deletes the item `ccx-NAME` or the key file. Without `--purge`, rm keeps the key and shows the command that deletes it.

## Add an API-key account

```sh
ccx manage add paid --api-key
ccx manage add paid --api-key --key-command 'pass show anthropic'
ccx manage add paid --api-key --key-file   # Keep the key in ~/.config/ccx/keys/paid (mode 0600).
op read 'op://vault/anthropic/key' | ccx manage add paid --api-key -y
```

`ccx manage add NAME --api-key` shows the plan and asks, as for a subscription account. Then it:

1. Reads the key with a hidden prompt, or one line of stdin when stdin is not a terminal.
2. Stores the key as the item `ccx-NAME` in the OS secret store: the macOS login keychain (through `security`) or the Linux Secret Service (through `secret-tool` from libsecret). These commands get the key only on stdin.
3. Makes the directory and runs sync, with no login and no copy of the preferences.
4. Writes the account to `config.toml` with `kind = "api_key"` and `key_store = "keyring"`, never the key.

With `--key-command COMMAND`, ccx keeps no copy of the key. It runs the command with `sh -c` at each paid launch and reads the key from stdout, for example from a password manager. Add runs it one time to test it. If the command fails or gives no output, ccx stops with an error. The entry gets `key_command = "COMMAND"`.

With no Secret Service (no D-Bus session, or no unlocked keyring), add shows the error and two fallbacks: `ccx manage add NAME --api-key --key-command '<command>'`, or a key file. On a terminal, add asks `Write the key to ~/.config/ccx/keys/NAME (mode 0600)?`. With no terminal, use `--key-file`. ccx writes the file only after you confirm. The file has mode 0600, in a directory with mode 0700. `--key-file` is the confirmation, and add does not try the OS secret store. The entry gets `key_store = "file"`. ccx refuses to read a key file with a different mode. To fix it, run `chmod 600 ~/.config/ccx/keys/NAME`.

## Show the spend of API-key accounts

```sh
ccx manage spend                         # This month, all accounts.
ccx manage spend --month 2026-09         # Local time.
ccx manage spend @paid                   # One account.
ccx manage spend --month 2026-09 --json  # Stable JSON for scripts.
```

The report estimates the monthly spend of each API-key account. Subscription accounts are not in it. Rows: each account and model, with account totals and a grand total. ccx:

1. Reads the session transcripts (`projects/**/*.jsonl`) in each API-key account directory.
2. Adds the `usage` token counts of each assistant message in the month for its model: input, output, cache write and cache read. It counts each message once, even when it has more than one transcript line.
3. Multiplies the token counts by the model price. Built-in prices cover the current Claude models. A `[pricing.<model>]` entry in `config.toml` sets the price of a model and replaces a built-in price (refer to [Configuration](configuration.md)). A name with a date suffix, for example `claude-sonnet-4-5-20250929`, uses the price of `claude-sonnet-4-5`.

Limits:

- The report is an estimate from local files, not your bill. ccx does not call a billing API or read the API key for it.
- A model with no price shows as `unpriced` with its token counts. The totals do not include it. To give it a price, add a `[pricing.<model>]` entry.
- All cache writes use the cache write price, also for the 1-hour cache and long prompts.
- ccx skips a line that is not valid JSON, or an assistant message with no timestamp, model or usage, and shows the number of skipped lines.
- After a handoff to an API-key account, the report counts the copied earlier messages from the month.

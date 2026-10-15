# API-key accounts (paid fallback)

An API-key account uses an Anthropic API key, not a subscription login (`kind = "api_key"` in `config.toml`). Each request costs money. Thus, ccx uses it only as a paid fallback:

```sh
ccx                         # All subscription accounts are limited: ccx asks first.
ccx --allow-paid            # Use the paid fallback with no prompt. Use this in scripts.
ccx --allow-paid @paid -p hi
ccx @paid --allow-paid -c   # --allow-paid can also come just after @name.
```

- When all subscription accounts are limited, ccx asks `All subscription accounts are limited. Use paid account NAME? [y/N]`. If you answer yes, ccx starts the paid account. If you answer no, ccx uses the soonest-reset rule (step 5 of [Automatic selection](launch.md#automatic-selection)).
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

## Add an API-key account

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

## Show the spend of API-key accounts

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
3. ccx multiplies the token counts by the price of the model. The built-in price table has the current Claude models. A `[pricing.<model>]` entry in `config.toml` replaces the built-in price of that model (refer to [Configuration](configuration.md)). A model name with a date suffix, for example `claude-sonnet-4-5-20250929`, also uses the price of `claude-sonnet-4-5`.

Important:

- The report is an estimate from local files. It is not your bill. ccx does not call a billing API and does not read the API key for the report.
- A model with no price shows as `unpriced` with its token counts. The totals do not include it. Add a `[pricing.<model>]` entry to give it a price.
- ccx uses the cache write price for all cache writes. It does not use a different price for the 1-hour cache or for long prompts.
- When a transcript line is not valid JSON, or an assistant message has no timestamp, model or usage, ccx skips the line. The report shows the number of skipped lines.
- `ccx handoff` copies a session with its earlier messages. When you hand off a session to an API-key account, the report also counts the copied messages that are in the month.

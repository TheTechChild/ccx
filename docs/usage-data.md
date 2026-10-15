# Usage data

ccx keeps a cache of the usage of each account. These commands show the usage and fetch it again.

```sh
ccx status                   # One row for each account.
ccx status --json            # The same data as stable JSON.
ccx usage refresh angel2     # Fetch the usage of one account.
ccx usage refresh --all      # Fetch the usage of all enabled accounts.
```

## Show the status of each account

```text
NAME    EMAIL             KIND  TIER     SESSION      WEEKLY      SCOPED                   SOURCE  AGE
angel2  work@example.com  max   Max 5x   1% (4h08m)   0% (6d6h)   ExampleModel 0% (6d6h)   /usage  0s
main    -                 max   Max 20x  11% (2h48m)  21% (3d2h)  ExampleModel 12% (3d2h)  /usage  2m
```

- `EMAIL`: from `config.local.toml`, or `-`.
- `KIND`: `subscriptionType` from `claude auth status`, or `api-key (paid)` with `-` in the usage columns.
- `TIER`: `oauthAccount.organizationRateLimitTier` from the `.claude.json` of the account. An unknown tier shows the raw value.
- `SESSION`, `WEEKLY`, `SCOPED`: the percent used and the time until reset. `reset` shows a past reset time: the data is from the last window, so ccx does not know the current percent. In the JSON, `reset` is `true` and `percent` is `null`.
- `SOURCE`, `AGE`: `/usage` or `cachedUsageUtilization`, and data age.

ccx uses a cache less than `usage.fresh_minutes` old (default 15). Else it runs the `/usage` fetches in parallel and waits a maximum of `usage.fetch_timeout_secs` (default 5), including the output read. A failed fetch uses `cachedUsageUtilization` from the `.claude.json` of the account, or shows `unknown`, and never makes the command fail.

Exit codes:

- 0: an enabled account is logged in and not limited.
- 3: all logged-in enabled accounts are limited: session usage is at least `pick.session_avoid` (default 90), or weekly usage at least `pick.weekly_avoid` (default 95).
- 4: no enabled account is logged in.

API-key accounts do not count. Unknown usage, `reset` and a scoped weekly window do not make an account limited (`ccx status` has no session model).

## Refresh the usage cache

`ccx usage refresh` writes `~/.local/state/ccx/usage/<name>.json` as a renamed temporary file (mode 0600), so readers never see a partial file. It waits a maximum of 30 seconds for each account. The exit code is 1 when a fetch fails.

## Automatic refresh

On an automatic launch (no `@name` and no `CCX_ACCOUNT`), ccx keeps the cache fresh, so most launches do not wait.

1. The usage data is the newer of the ccx cache and `cachedUsageUtilization`. ccx uses data less than `usage.fresh_minutes` old.
2. For older or missing data, ccx runs a blocking fetch for that account. Its limit, `usage.fetch_timeout_secs`, includes the wait for the lock. On timeout, ccx uses the old data or `cachedUsageUtilization` and continues the launch.
3. ccx starts `ccx usage refresh --all --background` as a detached, silent process, then starts `claude`.

All fetches use one file lock: `~/.local/state/ccx/refresh.lock`. `~/.local/state/ccx/refresh.json` records the start time of the last fetch.

The background refresh exits with no fetch when a different process holds the lock, or when the last fetch started less than `usage.refresh_min_gap_secs` (default 60) ago (the 60-second rule). The blocking fetch waits within its time limit, then reads the cache again. `ccx usage refresh` and `ccx status` wait. Only the background refresh uses the 60-second rule.

ccx releases the lock before it starts the refresh and `claude`. `ccx @name` fetches nothing and starts no refresh. Set `CCX_NO_REFRESH=1` to stop the blocking fetch and the background refresh of a launch.

## Background service

The service is optional (stale-while-revalidate works without it). It runs `ccx usage refresh --all --background --scheduled` every 60 seconds and fetches only the due accounts ([Adaptive interval](#adaptive-interval)).

```sh
ccx manage service install     # Write the job for this system and load it.
ccx manage service status      # Show the job, the last refresh, and the cache age and next fetch of each account.
ccx manage service uninstall   # Stop the job and remove its files.
```

| System | What `install` does |
|---|---|
| macOS | Writes `~/Library/LaunchAgents/io.github.thetechchild.ccx.refresh.plist` with `RunAtLoad` and `StartInterval` 60, and loads it with `launchctl bootstrap`. |
| Linux with systemd | Writes `ccx-refresh.service` and `ccx-refresh.timer` in `~/.config/systemd/user/` and runs `systemctl --user enable --now ccx-refresh.timer`. Without linger, the timer runs only while you are logged in, so it tells you to run `loginctl enable-linger`. |
| Linux with no systemd | Prints a crontab line (each minute) to add with `crontab -e`. ccx never edits the crontab and changes no files. |

- The job uses the absolute paths of ccx and `claude` (`install` puts the `claude` path in `CCX_CLAUDE_BIN`). The `PATH` of the job is the directory of `claude`, then `/usr/bin:/bin:/usr/sbin:/sbin` ([ADR 0007](adr/0007-m0-manual-tests.md)).
- When `claude` moves (for example, a mise upgrade), the job cannot find it, and `ccx manage service status` and `ccx manage doctor` show an error. Run `install` again.
- The job is a background refresh with the lock and the 60-second rule, so a job run and a launch never fetch at the same time.
- A second `install` with no changes does nothing. `status` works with no service installed, and exits 1 when a path in the job is missing.

### Adaptive interval

Each job run reads the usage cache and gives each account an interval. An account is due when its cache is older than its interval. When no account is due, the job starts no `claude` process. The interval comes from:

- The highest percent in `limits[]`: `usage.max_interval_secs` at 0%, `usage.min_interval_secs` at 90% or more, proportional between.
- An increase since the fetch before: at most half the time to 90% at that rate.
- The soonest `resets_at`: ends at the reset of a window above 0%. After a reset, `usage.min_interval_secs`.
- A ccx launch on the account in the last hour: half.

The interval stays between `usage.min_interval_secs` (default 120) and `usage.max_interval_secs` (default 1800). A failed fetch tries again after `usage.min_interval_secs`. Set `usage.adaptive = false` to fetch each account every 300 seconds.

The job writes the next due time of each account to `~/.local/state/ccx/schedule.json`. Each launch writes its time to `~/.local/state/ccx/launches/<name>`. Example `ccx manage service status` line: `main    7m old, next fetch: 14:25 (in 3m, every 10m)`.

## The `/usage` fetch

ccx runs this command for each account ([ADR 0005](adr/0005-m0-usage-hooks-off.md)):

```sh
claude -p "/usage" --output-format stream-json --verbose --no-session-persistence --strict-mcp-config --settings '{"disableAllHooks":true}'
```

- It always runs in `~/.local/state/ccx/probe` (mode 0700, made if missing), never in your current directory. Project settings, project hooks, project MCP servers and project `CLAUDE.md` files have no effect.
- `CLAUDE_CONFIG_DIR` is the account directory, not set for the base account.
- All hooks are off. `--no-session-persistence` stops the session file.
- ccx reads `usage_report.rate_limits.limits[]` from the `assistant` event.
- ccx never reads, keeps or refreshes Claude Code credentials. A test fails if the code uses the OAuth token endpoint, reads `.credentials.json` or a keychain secret.

# Usage data

## Show the status of each account

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

## Refresh the usage cache

```sh
ccx usage refresh angel2   # Fetch the usage of one account.
ccx usage refresh --all    # Fetch the usage of all enabled accounts.
```

The refresh writes `~/.local/state/ccx/usage/<name>.json`. ccx writes a temporary file at mode 0600, then renames it. Thus, a reader never sees a partial file. The refresh waits for a maximum of 30 seconds for each account. The exit code is 1 when a fetch fails.

## Automatic refresh

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

## Background service

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

- The job uses the absolute path of ccx and the absolute path of `claude`. ccx finds the `claude` path when you run `install`, and puts it in `CCX_CLAUDE_BIN`. The `PATH` of the job is the directory of `claude`, then `/usr/bin:/bin:/usr/sbin:/sbin` ([ADR 0007](adr/0007-m0-manual-tests.md)).
- When `claude` moves, for example after a mise upgrade, the job cannot find it. `ccx manage service status` and `ccx manage doctor` then show an error. Run `ccx manage service install` again.
- The job is a background refresh. It uses the same lock and the same `usage.refresh_min_gap_secs` rule as the refresh of a launch. Thus, a job run and a launch never fetch at the same time.
- A second `install` with no changes does nothing. `status` works also when no service is installed. Its exit code is 1 when a path in the job does not exist.
- The crontab line runs each minute.

### Adaptive interval

Each run of the job reads the usage cache. For each account, it calculates an interval. The account is due when its cache is older than its interval. The job fetches only the due accounts. When no account is due, the job starts no `claude` process. These rules set the interval:

| Data | Effect on the interval |
|---|---|
| The highest percent in `limits[]` | At 0%, the interval is `usage.max_interval_secs`. At 90% or more, it is `usage.min_interval_secs`. Between these values, the interval is proportional. |
| The change since the fetch before | When the percent increases, the interval is at most half of the time until the account gets to 90% at that rate. |
| The soonest `resets_at` | The interval stops at the reset of a window that is not at 0%. When a window has reset, the interval is `usage.min_interval_secs`. |
| The last ccx launch on the account | When the last launch is less than one hour ago, the interval is half. |

The interval is always between `usage.min_interval_secs` (default 120) and `usage.max_interval_secs` (default 1800). When a fetch fails, the job tries again after `usage.min_interval_secs`. Set `usage.adaptive = false` to fetch each account every 300 seconds.

The job writes the next due time of each account to `~/.local/state/ccx/schedule.json`. Each launch writes its time to `~/.local/state/ccx/launches/<name>`. `ccx manage service status` shows the next fetch of each account, for example `main    7m old, next fetch: 14:25 (in 3m, every 10m)`.

## The `/usage` fetch

ccx runs this command for each account ([ADR 0005](adr/0005-m0-usage-hooks-off.md)):

```sh
claude -p "/usage" --output-format stream-json --verbose --no-session-persistence --strict-mcp-config --settings '{"disableAllHooks":true}'
```

- The command always runs in `~/.local/state/ccx/probe`. ccx makes this directory at mode 0700 if it does not exist. The command never runs in your current directory. Thus, project settings, project hooks, project MCP servers and project `CLAUDE.md` files have no effect.
- For the base account, `CLAUDE_CONFIG_DIR` is not set. For the other accounts, `CLAUDE_CONFIG_DIR` is the directory of the account.
- All hooks are off. `--no-session-persistence` stops the session file.
- ccx reads `usage_report.rate_limits.limits[]` from the `assistant` event.
- ccx never reads, keeps or refreshes Claude Code credentials. A test fails if the code uses the OAuth token endpoint, reads `.credentials.json` or reads a keychain secret.

# 0005. M0 test 1: hooks stay off for the `/usage` fetch

- **Status:** Accepted
- **Date:** 2026-10-12
- **Claude Code version:** 2.1.296
- **Platform:** macOS (Darwin 25)
- **Issue:** #2, test 1

## Context

ccx gets usage data with `claude -p "/usage"` (refer to [ADR 0004](0004-usage-source.md)). The accounts have hooks in `settings.json`. Some hooks start on `SessionStart` and `SessionEnd`. A usage fetch must not start these hooks, because:

- Hooks can be slow. They make the fetch slower.
- Hooks can have side effects, for example notifications or log entries, each time ccx refreshes the usage.

Thus, ccx must turn off all hooks for the fetch. Test 1 examines if this command turns off the hooks and gives the usage data:

```sh
claude -p "/usage" --output-format stream-json --verbose \
  --no-session-persistence --strict-mcp-config \
  --settings '{"disableAllHooks":true}'
```

## Test procedure

1. Make an empty temporary directory and go to it (a neutral cwd).
2. Run the command for the two real accounts, with a 60-second timeout (`perl -e 'alarm 60; exec @ARGV' claude ...`):
   - base account: `env -u CLAUDE_CONFIG_DIR claude ...`
   - angel2 account: `CLAUDE_CONFIG_DIR=$HOME/.claude-angel2 claude ...`
3. Count the events with `"hook_` in the output (`hook_started` and `hook_response`).
4. Examine the shape of the events with `jq`. Record only the keys and the types, not the values.

Before the test, `settings.json` (shared by the two accounts through a symlink) had hooks for 14 events, which include `SessionStart`, `SessionEnd`, `UserPromptSubmit` and `Stop`.

## Results

| Check | base | angel2 |
|---|---|---|
| Exit code | 0 | 0 |
| Time (`duration_ms` in the result event) | 346 ms | 920 ms |
| Output events | `system/init`, `assistant`, `result/success` | same |
| `hook_started` or `hook_response` events | 0 | 0 |
| `usage_report.rate_limits.limits[]` is present | yes, 3 items | yes, 3 items |
| `num_turns` and `total_cost_usd` in the result event | 0 and 0 | 0 and 0 |
| stderr | empty | empty |

**Result: PASS.** No hook runs. The assistant event contains `usage_report.rate_limits.limits[]`. The fetch makes no model request and costs nothing.

### Event shape

The `usage_report` object is a top-level key of the `assistant` event. It is **not** in `message`. The `message` has `model: "<synthetic>"` and one `text` content block.

Synthetic sample (the values are examples, not real data):

```json
{
  "type": "assistant",
  "session_id": "00000000-0000-4000-8000-000000000000",
  "uuid": "00000000-0000-4000-8000-000000000001",
  "parent_tool_use_id": null,
  "timestamp": "2026-01-01T00:00:00.000Z",
  "local_command_run": { "command": "usage", "args": "" },
  "local_command_source": "<local-command-stdout>Current session: 5% used ...</local-command-stdout>",
  "message": {
    "id": "00000000-0000-4000-8000-000000000002",
    "type": "message",
    "role": "assistant",
    "model": "<synthetic>",
    "content": [{ "type": "text", "text": "..." }],
    "stop_reason": "end_turn"
  },
  "usage_report": {
    "rate_limits": {
      "limits": [
        {
          "kind": "session",
          "group": "session",
          "percent": 5,
          "resets_at": "2026-01-01T05:00:00.000000+00:00",
          "severity": "normal",
          "is_active": false,
          "scope": null
        },
        {
          "kind": "weekly_all",
          "group": "weekly",
          "percent": 11,
          "resets_at": "2026-01-04T06:00:00.000000+00:00",
          "severity": "normal",
          "is_active": true,
          "scope": null
        },
        {
          "kind": "weekly_scoped",
          "group": "weekly",
          "percent": 0,
          "resets_at": "2026-01-04T06:00:00.000000+00:00",
          "severity": "normal",
          "is_active": false,
          "scope": { "model": { "display_name": "ExampleModel" }, "surface": null }
        }
      ],
      "extra_usage": {
        "is_enabled": false,
        "monthly_limit": null,
        "used_credits": null,
        "utilization": null,
        "currency": null
      }
    },
    "session": {
      "model_usage": {},
      "total_cost_usd": 0,
      "total_duration_ms": 0,
      "total_api_duration_ms": 0,
      "total_lines_added": 0,
      "total_lines_removed": 0
    }
  }
}
```

Field types that the parser must accept:

| Field | Type | Notes |
|---|---|---|
| `kind` | string | Values seen: `session`, `weekly_all`, `weekly_scoped`. |
| `group` | string | Values seen: `session`, `weekly`. |
| `percent` | number | Integer values seen. Parse as a float. |
| `resets_at` | string or **null** | RFC 3339 with microseconds and `+00:00`. It is `null` when the window has not started (seen on a session limit at 0%). |
| `severity` | string | Value seen: `normal`. |
| `is_active` | boolean | |
| `scope` | object or null | `null` for `session` and `weekly_all`. For `weekly_scoped`: `scope.model.display_name` (string) and `scope.surface` (null in the test). |

## Decision

- ccx uses the exact command in the Context section for the usage fetch. All the flags work together.
- ccx reads `usage_report` from the top level of the first `assistant` event.
- ccx accepts `resets_at: null`. A limit with no reset time has no active window.
- ccx ignores unknown `kind` values and unknown fields, so that new versions of Claude Code do not break the fetch.

## Consequences

- No D4 fallback is necessary. Test 1 passed.
- #9 (`/usage` fetch): use this command and this shape. Write a fixture with `resets_at: null` for a parser test.
- #9: `--no-session-persistence` does not stop all writes. The fetch makes the directory `projects/<encoded cwd>/memory/` (empty) in the account directory. Thus, ccx must run the fetch from one fixed neutral directory, for example `~/.local/state/ccx/probe`, so that only one such directory exists.
- #10 (selection): the scoped limit has the model name in `scope.model.display_name`. This agrees with D5.

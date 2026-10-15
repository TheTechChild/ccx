# Configuration

ccx reads `config.toml` from the first location that applies: `$CCX_CONFIG`, `$XDG_CONFIG_HOME/ccx/config.toml`, or `~/.config/ccx/config.toml` (also on macOS).

```toml
[launch]                   # Optional.
claude_bin = "~/.local/bin/claude"   # Optional. The default is the claude on PATH.

[base]                     # Optional. Used by `ccx manage bootstrap`.
repo = "https://github.com/example/claude-config.git"   # The git repo of the base account ~/.claude.

[accounts.main]
dir = "~/.claude"          # The base account. Exactly one account must use this directory.

[accounts.angel2]
dir = "~/.claude-angel2"
aliases = ["claude2"]      # Optional. `ccx manage shell-init` makes `alias claude2='ccx @angel2'`.
own = ["statusline.py"]    # Optional. Items that `ccx manage sync` does not link.
weight = 5                 # Optional. Overrides the tier weight for automatic selection.
enabled = true             # Optional. false: automatic selection does not use the account.

[accounts.paid]            # Optional. An API-key account: a paid fallback.
dir = "~/.claude-paid"
kind = "api_key"           # "subscription" (the default) or "api_key".
key_store = "keyring"      # "keyring" (the OS secret store item `ccx-paid`) or "file" (keys/paid, next to config.local.toml).
# key_command = "pass show anthropic"  # Or: a command that prints the key. Use key_store or key_command, not both.

[pick]                     # Optional. Usage (%) that makes an account limited:
session_avoid = 90         # session usage.
weekly_avoid = 95          # weekly usage.
scoped_avoid = 98          # matched scoped weekly usage.
stickiness = 10            # Points within the best score that keep the last account.

[usage]                    # Optional. The limits of the automatic refresh.
fresh_minutes = 15         # Usage data that is less old than this is fresh.
fetch_timeout_secs = 5     # The time limit of the blocking fetch of a launch.
refresh_min_gap_secs = 60  # The minimum time between two background refreshes.
adaptive = true            # Adapt the service interval to the usage. false: every 300 seconds.
min_interval_secs = 120    # The shortest service interval. It must be 1 or more.
max_interval_secs = 1800   # The longest service interval. It must be min_interval_secs or more.

[prefs]                    # Optional. Changes to the allowlist of `ccx manage sync --prefs`.
allow = []                 # Keys to copy also.
deny = []                  # Keys not to copy.

[sync]                     # Optional. Top-level entries of ~/.claude that are not built-in shared items.
share = []                 # Entries that `ccx manage sync` links like a built-in shared item.
ignore = []                # Entries that sync does not share and does not show as UNKNOWN.

[pricing.claude-opus-5-5]  # Optional. Sets the price of a model for `ccx manage spend`, over a built-in price.
input = 4                  # USD for each million tokens: input,
output = 20                # output,
cache_write = 5            # cache write
cache_read = 0.2           # and cache read.
```

A `[pricing.<model>]` entry must have all four prices. Each price is 0 or more.

A name in `[sync] share` or `[sync] ignore` is a top-level entry of `~/.claude`. It must not be empty, `.` or `..`. It must not contain `/` or start with `.`. It must not be on the never-share list, be a built-in shared item, or be `skills`. A name cannot be in the two lists. A `share` name that is not in `~/.claude` has no effect.

Put email addresses only in the overlay `config.local.toml`. ccx reads it from `$XDG_CONFIG_HOME/ccx/`, else from `~/.config/ccx/`. ccx merges it over `config.toml`. Do not commit the overlay. Give it mode 0600. Key files of API-key accounts are in `keys/` in the same directory, for example `~/.config/ccx/keys/paid`.

```toml
[accounts.angel2]
email = "work@example.com"
```

[Issue #3](https://github.com/TheTechChild/ccx/issues/3) has the full schema.

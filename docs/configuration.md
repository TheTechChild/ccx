# Configuration

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

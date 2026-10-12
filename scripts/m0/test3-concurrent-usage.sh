#!/usr/bin/env bash
# M0 test 3: is it safe to fetch /usage while a session in the same account
# is open? Both processes write .claude.json.
#
# Usage: scripts/m0/test3-concurrent-usage.sh [account-dir] [fetch-count]
# Default account-dir: $HOME/.claude-angel2. Default fetch-count: 5.
#
# The script prints only key names and counts, never values.
set -euo pipefail

dir=${1:-$HOME/.claude-angel2}
count=${2:-5}
json=$dir/.claude.json
probe=$(mktemp -d)
trap 'rm -rf "$probe"' EXIT

command -v jq >/dev/null || { echo "jq is necessary (mise use -g jq)." >&2; exit 2; }
[ -f "$json" ] || { echo "Not found: $json" >&2; exit 2; }

# keys FILE: sorted top-level keys, or "INVALID" if the JSON does not parse.
keys() {
  jq -r 'keys[]' "$1" 2>/dev/null | sort || echo INVALID
}

fetch() {
  (cd "$probe" && CLAUDE_CONFIG_DIR=$dir perl -e 'alarm 60; exec @ARGV' \
    claude -p "/usage" --output-format stream-json --verbose \
    --no-session-persistence --strict-mcp-config \
    --settings '{"disableAllHooks":true}') |
    jq -e 'select(.type == "assistant") | .usage_report.rate_limits.limits | length > 0' \
      >/dev/null 2>&1
}

keys "$json" >"$probe/keys-before"
projects_before=$(jq '.projects // {} | length' "$json")
echo "BEFORE: $(wc -l <"$probe/keys-before" | tr -d ' ') top-level keys, $projects_before projects"

cat <<EOF

Do these steps in a different terminal:
  1. Go to a project directory and run:  CLAUDE_CONFIG_DIR=$dir claude
  2. Send one short prompt, for example "say hi". Keep the session open.

Then come back here and push Enter. The script runs $count /usage fetches.
EOF
read -r _

fail=0
for i in $(seq 1 "$count"); do
  if fetch; then fetch_ok=yes; else fetch_ok=no; fail=1; fi
  if jq empty "$json" 2>/dev/null; then valid=yes; else valid=no; fail=1; fi
  echo "fetch $i: usage data=$fetch_ok, .claude.json valid=$valid"
  sleep 2
done

cat <<EOF

Now, in the open session:
  1. Send one more prompt, for example "say bye".
  2. Type /exit.

Then come back here and push Enter.
EOF
read -r _

if ! jq empty "$json" 2>/dev/null; then
  echo "RESULT: FAIL. .claude.json is not valid JSON."
  exit 1
fi
keys "$json" >"$probe/keys-after"
projects_after=$(jq '.projects // {} | length' "$json")
lost=$(comm -23 "$probe/keys-before" "$probe/keys-after")
echo "AFTER:  $(wc -l <"$probe/keys-after" | tr -d ' ') top-level keys, $projects_after projects"
if [ -n "$lost" ]; then
  echo "Top-level keys that are gone:"
  printf '  %s\n' "$lost"
  fail=1
fi
if [ "$projects_after" -lt "$projects_before" ]; then
  echo "The number of projects decreased."
  fail=1
fi
echo
if [ "$fail" -eq 0 ]; then
  echo "RESULT: PASS. It is safe to fetch /usage while a session is open."
else
  echo "RESULT: FAIL. No D4 fallback applies: STOP and tell Claude."
fi
echo "claude version: $(claude --version 2>/dev/null || echo unknown)"

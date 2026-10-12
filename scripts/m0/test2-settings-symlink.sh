#!/usr/bin/env bash
# M0 test 2: does the settings.json symlink in the second account stay a
# symlink after a /config write in that account?
#
# Usage: scripts/m0/test2-settings-symlink.sh [account-dir]
# Default account-dir: $HOME/.claude-angel2
#
# The script records the state, asks you to change a setting with /config,
# then records the state again. It prints no file contents.
set -euo pipefail

dir=${1:-$HOME/.claude-angel2}
file=$dir/settings.json

state() {
  if [ -L "$file" ]; then
    echo "symlink -> $(readlink "$file")"
  elif [ -f "$file" ]; then
    echo "regular file"
  else
    echo "missing"
  fi
}

checksum() {
  # Checksum of the file that the path resolves to (no contents shown).
  if [ -e "$file" ]; then
    shasum -a 256 "$file" | cut -c1-16
  else
    echo "-"
  fi
}

before=$(state)
sum_before=$(checksum)
echo "BEFORE: $file is a $before (sha256 prefix $sum_before)"
case $before in
  symlink*) ;;
  *)
    echo "The file is not a symlink now. Test 2 needs a symlink at the start. Stop." >&2
    exit 2
    ;;
esac

cat <<EOF

Do these steps in a different terminal:
  1. Run:  CLAUDE_CONFIG_DIR=$dir claude
  2. Type /config and change one setting that Claude Code keeps in
     settings.json. Example: change "Auto-compact" or "Verbose output".
  3. Change the same setting back to its first value.
  4. Type /exit.

Then come back here and push Enter.
EOF
read -r _

after=$(state)
sum_after=$(checksum)
echo "AFTER:  $file is a $after (sha256 prefix $sum_after)"
echo
if [ "$after" = "$before" ]; then
  echo "RESULT: PASS. The symlink is kept."
else
  echo "RESULT: FAIL. The symlink was replaced ($after)."
  echo "D4 fallback: 'ccx manage sync' (#7) reports CONFLICT and relinks only with --force."
fi
if [ "$sum_before" = "$sum_after" ]; then
  echo "NOTE: the checksum did not change. Make sure that /config wrote the file"
  echo "      (change a setting, do not change it back, then run the test again)."
fi
echo "claude version: $(claude --version 2>/dev/null || echo unknown)"

#!/usr/bin/env bash
# M0 test 5: does `claude --resume <id>` work in account B after we copy all
# session files from account A? The file set comes from ADR 0006.
#
# Usage:
#   scripts/m0/test5-resume-copy.sh latest [src-dir]
#       Print the id of the newest session for the current directory.
#   scripts/m0/test5-resume-copy.sh copy <id> [src-dir] [dst-dir]
#       Copy the session files. It never overwrites a file.
#   scripts/m0/test5-resume-copy.sh undo <id> [dst-dir]
#       Remove the copied files from dst-dir again.
#
# Defaults: src-dir $HOME/.claude, dst-dir $HOME/.claude-angel2.
set -euo pipefail

state_dir=$HOME/.local/state/ccx-m0
die() { echo "$*" >&2; exit 2; }

# transcript SRC ID: path of projects/*/<id>.jsonl in SRC.
transcript() {
  local f
  for f in "$1"/projects/*/"$2".jsonl; do
    [ -f "$f" ] && { echo "$f"; return; }
  done
  die "No transcript for session $2 in $1/projects."
}

# session_cwd FILE: the first cwd field of the transcript.
session_cwd() {
  grep -o '"cwd":"[^"]*"' "$1" | head -n 1 | sed 's/^"cwd":"//; s/"$//'
}

# is_open SRC ID: true if a live process has this session open.
is_open() {
  local f pid
  for f in "$1"/sessions/*.json; do
    [ -f "$f" ] || continue
    if grep -q "\"sessionId\":\"$2\"" "$f"; then
      pid=$(basename "$f" .json)
      kill -0 "$pid" 2>/dev/null && return 0
    fi
  done
  return 1
}

resume_cmd() {
  local dst=$1 id=$2
  if [ "$dst" = "$HOME/.claude" ]; then
    echo "env -u CLAUDE_CONFIG_DIR claude --resume $id"
  else
    echo "CLAUDE_CONFIG_DIR=$dst claude --resume $id"
  fi
}

cmd_latest() {
  local src=${1:-$HOME/.claude} here
  here=$(pwd -P)
  local best="" f
  # Find the newest transcript with this cwd.
  for f in "$src"/projects/*/*.jsonl; do
    [ -f "$f" ] || continue
    if [ "$(session_cwd "$f")" = "$here" ] && { [ -z "$best" ] || [ "$f" -nt "$best" ]; }; then
      best=$f
    fi
  done
  [ -n "$best" ] && { basename "$best" .jsonl; return; }
  die "No session for $here in $src."
}

cmd_copy() {
  local id=${1:?session id is necessary} src=${2:-$HOME/.claude} dst=${3:-$HOME/.claude-angel2}
  local t enc age now mtime slug rel
  local -a items=()

  [ "$src" != "$dst" ] || die "Source and target are the same."
  t=$(transcript "$src" "$id")
  enc=$(basename "$(dirname "$t")")

  is_open "$src" "$id" && die "Session $id is open in $src. Close it first."
  now=$(date +%s)
  mtime=$(stat -f %m "$t" 2>/dev/null || stat -c %Y "$t")
  age=$((now - mtime))
  [ "$age" -ge 10 ] || die "The transcript changed $age s ago. Wait and try again."

  items+=("projects/$enc/$id.jsonl")
  for d in "$src"/projects/*/"$id"; do
    [ -d "$d" ] && items+=("${d#"$src"/}")
  done
  [ -d "$src/file-history/$id" ] && items+=("file-history/$id")
  [ -d "$src/session-env/$id" ] && items+=("session-env/$id")
  while IFS= read -r slug; do
    [ -n "$slug" ] && [ -f "$src/plans/$slug.md" ] && items+=("plans/$slug.md")
  done < <(grep -o '"slug":"[^"]*"' "$t" | sed 's/^"slug":"//; s/"$//' | sort -u)

  echo "Copy from $src to $dst:"
  for rel in "${items[@]}"; do
    [ -e "$dst/$rel" ] && die "Already present in target: $dst/$rel. Run 'undo' first."
    echo "  $rel"
  done
  printf 'Continue? [y/N] '
  read -r answer
  [ "$answer" = y ] || die "Stopped."

  mkdir -p "$state_dir"
  : >"$state_dir/test5-$id.txt"
  for rel in "${items[@]}"; do
    mkdir -p "$(dirname "$dst/$rel")"
    cp -Rp "$src/$rel" "$dst/$rel"
    echo "$rel" >>"$state_dir/test5-$id.txt"
  done

  cat <<EOF

Copied. Now do these steps:
  1. cd "$(session_cwd "$t")"
  2. $(resume_cmd "$dst" "$id")
  3. Make sure that the earlier messages are shown. Ask a question about them,
     for example "Which word did I ask you to remember?".
  4. Type /exit.
Record: resume worked (yes/no), and any error text.
To remove the copy: scripts/m0/test5-resume-copy.sh undo $id $dst
EOF
}

cmd_undo() {
  local id=${1:?session id is necessary} dst=${2:-$HOME/.claude-angel2}
  local manifest=$state_dir/test5-$id.txt rel
  [ -f "$manifest" ] || die "No record of a copy: $manifest"
  while IFS= read -r rel; do
    case $rel in
      projects/* | file-history/* | session-env/* | plans/*) rm -rf "${dst:?}/$rel" ;;
      *) die "Unexpected path in $manifest: $rel" ;;
    esac
    echo "removed $dst/$rel"
  done <"$manifest"
  rm -f "$manifest"
}

case ${1:-} in
  latest) shift; cmd_latest "$@" ;;
  copy) shift; cmd_copy "$@" ;;
  undo) shift; cmd_undo "$@" ;;
  *) sed -n '2,14p' "$0"; exit 2 ;;
esac

#!/usr/bin/env bash
# Privacy check: fail on each email address that does not end in
# "@example.com" and does not contain "noreply".
#
# It scans all tracked files, and the author email, committer email and
# message of every commit. Usage: check-emails.sh [repo-dir]
set -euo pipefail

cd "${1:-.}"

email_re='[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}'
found=0

allowed() {
  local lower
  lower=$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')
  case "$lower" in
    *@example.com | *noreply*) return 0 ;;
    *) return 1 ;;
  esac
}

# check_text WHERE TEXT: report each address in TEXT that is not allowed.
check_text() {
  local where=$1 text=$2 email
  while IFS= read -r email; do
    [ -n "$email" ] || continue
    if ! allowed "$email"; then
      printf '%s: %s\n' "$where" "$text"
      found=1
      return
    fi
  done < <(printf '%s\n' "$text" | grep -oE "$email_re" || true)
}

# Tracked files. git grep prints "file:line:text".
while IFS=: read -r file line text; do
  check_text "$file:$line" "$text"
done < <(git grep -n -I -E -e "$email_re" || true)

# Commits.
while IFS= read -r commit; do
  short=$(git rev-parse --short "$commit")
  check_text "commit $short author" "$(git show -s --format=%ae "$commit")"
  check_text "commit $short committer" "$(git show -s --format=%ce "$commit")"
  n=0
  while IFS= read -r text; do
    n=$((n + 1))
    check_text "commit $short message:$n" "$text"
  done < <(git show -s --format=%B "$commit")
done < <(git rev-list --all)

if [ "$found" -ne 0 ]; then
  echo "check-emails: FAIL. Use *@example.com placeholders only." >&2
  exit 1
fi
echo "check-emails: OK"

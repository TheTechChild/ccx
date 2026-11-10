# 0016. Sync never shares `README.md`

- **Status:** Accepted
- **Date:** 2026-11-09
- **Issue:** #74
- **Changes:** the never-share list of [ADR 0008](0008-sync-share-lists.md)

## Context

A user can keep the base directory `~/.claude` in a git repo, so that the portable config moves to a new machine. Such a repo often has a `README.md`. The file documents the repo. It is not part of the setup of an account.

`README.md` was in no group of [ADR 0008](0008-sync-share-lists.md). Sync and doctor showed it as `UNKNOWN`, and doctor showed an INFO line on each run. One user deleted the file only to stop that line.

## Decision

- `README.md` is on the never-share list. Sync does not link it, and sync and doctor do not show it as unknown.
- `README.md` is on the `MOVED` list of [ADR 0015](0015-sync-rules-and-runtime-items.md). A config that has `README.md` in `[sync] ignore` still loads. Doctor shows an INFO line that the entry is not necessary.

## Consequences

- A `README.md` in the base directory causes no doctor line.
- A user cannot share `README.md` with `[sync] share`. The other accounts do not need the documentation of the repo.

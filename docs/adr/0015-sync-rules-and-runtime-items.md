# 0015. Sync shares `rules/` and never shares the daemon and transcript items

- **Status:** Accepted
- **Date:** 2026-11-08
- **Issue:** #72
- **Changes:** the share lists of [ADR 0008](0008-sync-share-lists.md)

## Context

[ADR 0008](0008-sync-share-lists.md) puts each top-level entry of the base directory into a group. New Claude Code versions write four entries that are in no group, so sync and doctor show them as `UNKNOWN`:

- `rules/`: user-level instruction files. Claude Code loads them into each session, like `CLAUDE.md`.
- `transcripts/`: session transcripts.
- `daemon.lock` and `daemon.status.json`: the lock and the supervisor pid of the background daemon. Each account has its own.

Sync shares `CLAUDE.md`, and a user-level `CLAUDE.md` often refers to its rule files. Without `rules/`, the other accounts get the reference but not the files. A user can share `rules/` with `[sync] share`, but each user must find this workaround.

## Decision

- `rules` is a built-in shared item. Sync links it as one item, as for `agents/`.
- `transcripts`, `daemon.lock` and `daemon.status.json` are on the never-share list.
- The `MOVED` list in `src/sync.rs` names these four items. Configs written before this change stay valid:
  - `[sync] share` and `[sync] ignore` can name a `MOVED` item.
  - `rules` in `ignore` is not shared. The user chose that before `rules` became a shared item.
  - A never-share item in `share` is not shared. The never-share list is a safety rule.
  - `ccx manage doctor` shows an INFO line for each `[sync]` entry that names a `MOVED` item.

## Consequences

- A new account gets `rules/` from `ccx manage add` with no prompt.
- Doctor and sync do not show the daemon and transcript items as unknown.
- When a later release puts more unknown items into a group, add them to `MOVED`. Then the configs that name them still load.

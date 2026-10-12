# 0003. Sharing model is "link all"

- **Status:** Accepted
- **Date:** 2026-10-12

## Context

The user keeps settings, skills, agents, commands and other setup items in the base directory `~/.claude`. Each account must have the same setup. If each account has its own copies, the copies become different over time.

## Decision

- `ccx manage sync` links each shareable item in the base directory into each account directory with a symbolic link.
- ccx links each skill one at a time. Thus, an account can also have local skills.
- Some items must not be shared, for example identity files, credentials, sessions and history. ccx keeps a "never share" list for these items.
- ccx sorts each item in the base directory into one of three groups: shared, never shared or unknown. `ccx manage doctor` reports unknown items.
- Sync never deletes through a symbolic link. If an item is in the way, sync moves it to a backup before it makes the link.

## Consequences

- A change in the base directory is immediately available in all accounts.
- A write from Claude Code into a linked file goes to the base file. M0 test 2 must make sure that a symbolic link to `settings.json` stays a link after a write.
- A new item type from Claude Code is "unknown" until a ccx release puts it in a group.

# 0009. Handoff forks with a ccx rewrite, not with `--fork-session`

- **Status:** Accepted
- **Date:** 2026-10-12
- **Claude Code version:** 2.1.296
- **Issue:** #14

## Context

`ccx handoff <id> @B` copies session `<id>` from account A into account B. Sometimes B has a copy of `<id>` already, from an earlier handoff. The re-handoff rule is:

- When the copy in B is a prefix of the transcript in A, the copy in B is older. ccx overwrites it.
- When it is not a prefix, the user continued the session in B. ccx must not change the copy in B. ccx writes a fork with a new session id and tells the user the new id.

Claude Code has the option `claude --resume <id> --fork-session`. `claude --help` says: "When resuming, create a new session ID instead of reusing the original". We examined if ccx can use this option to make the fork.

## Decision

ccx makes the fork itself:

1. ccx makes a new random session id (UUID version 4).
2. ccx copies the source files to the names with the new id: `projects/<enc>/<new>.jsonl`, each `projects/*/<new>/`, `file-history/<new>/` and `session-env/<new>/`.
3. In each `.jsonl` file of the fork (the transcript and the subagent transcripts), ccx changes the `"sessionId":"<id>"` field to `"sessionId":"<new>"`. Claude Code writes compact JSON. Text in a JSON string has escaped quotes, so the pattern agrees only with the field.
4. ccx does not change a `plans/<slug>.md` file that is in B already and is different. ccx shows a warning.
5. ccx execs `claude --resume <new>` in B.

ccx does not use `--fork-session`.

## Reasons

- `--fork-session` works only on a session that claude can find. claude finds `<id>` in B only at `projects/<enc>/<id>.jsonl`. That file is the copy that ccx must keep. To give claude the version from A, ccx must write it into B with a different name. Thus, ccx must rename the files in all cases.
- `--fork-session` makes the new id only after claude starts. ccx cannot tell the user the new id before the exec, and `--no-resume` cannot print it.
- The ccx rewrite is a pure file operation. A test with a temporary account directory and a fake `claude` can check it. A test of `--fork-session` needs a logged-in account.

## Consequences

- A fork has the full transcript of A with a new id. The copy in B stays the same.
- If Claude Code adds a field that also holds the session id, the fork keeps the old value in that field. Examine the transcript format when the Claude Code version changes.

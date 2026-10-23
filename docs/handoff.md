# Move a session to a different account

When one account reaches its limit, `ccx handoff` copies a session to a different account and resumes it there.

```sh
ccx handoff                       # List threads of this directory from all accounts. On a terminal, select a thread and target.
ccx handoff --all                 # List threads from all directories.
ccx handoff <id> @angel2          # Copy <id> into angel2 and run `claude --resume <id>` there. No prompts.
ccx handoff <id> @angel2 --no-resume   # Copy only. Print the session id in the target.
```

The list shows the session id, account, time of the last change and first prompt. When stdin or stderr is not a terminal, `ccx handoff` prints the list and stops. `<id>` can be a unique start of 8 or more characters.

## What handoff copies

- `projects/<dir>/<id>.jsonl`, the transcript. `<dir>` is the encoded session directory.
- Each `projects/*/<id>/` sidecar directory.
- If present: `file-history/<id>/`, `session-env/<id>/` and `plans/<slug>.md` for each `slug` in the transcript.

It does not copy `todos/`, `projects/<dir>/memory/` or `jobs/`. It never moves, deletes or changes a source file.

Handoff refuses when the session is open:

- A `sessions/<pid>.json` in the source or target has the session id, its process is alive, and its start time agrees with `procStart`. Handoff never reads `sessions/*.key`.
- The transcript changed in the last 5 seconds.

When the target copy is a prefix of the source transcript, handoff overwrites it. Otherwise, handoff keeps it, writes a fork with a new session id and shows that id ([ADR 0009](adr/0009-handoff-fork-by-rewrite.md)).

When `oauthAccount.organizationUuid` in `.claude.json` differs between the accounts, handoff warns before the copy: Claude cannot use the earlier thinking in the target, and the next responses can be slower and use more tokens.

After the copy, ccx runs `claude --resume <id>` in the target from the session directory, with the environment rules of `ccx @name`.

## Automatic handoff on `ccx -c` and `ccx --resume <id>`

`ccx -c`, `ccx --continue` and `ccx --resume <id>` (or `-r <id>`, `--resume=<id>`) find the session owner from the files on disk, also for sessions started with plain `claude` or an alias such as `claude2`:

- `--resume <id>`: the account with `projects/*/<id>.jsonl`. The newest copy wins.
- `-c`: the account with the newest `*.jsonl` in `projects/<encoded cwd>/`. With none, ccx uses automatic selection.

[Automatic selection](launch.md#automatic-selection) defines "limited" and picks the target. When the session stopped at a usage limit, the owner is limited until the reset, also when the usage cache is old ([Limit hits](launch.md#automatic-selection)). Then:

1. **Owner not limited:** ccx launches the owner and copies nothing.
2. **Owner limited:** ccx hands off to the target and resumes there. `-c` becomes `--resume <id>`. For a fork, ccx resumes the fork.
3. **Owner is an [API-key account](api-key-accounts.md) and a subscription account is not limited:** ccx hands off to the target without a prompt.
4. **All subscription accounts limited:** ccx asks `All subscription accounts are limited. Use paid account NAME? [y/N]`. NAME is the API-key owner, else the first API-key account. `--allow-paid` skips the prompt.
   - Yes: ccx resumes on the paid account, with a handoff from a subscription owner.
   - No, or no terminal and no `--allow-paid`: a subscription owner keeps the session, with a warning. An API-key owner hands off to the subscription account with the soonest reset.
5. **Handoff refuses** (for example, an open session): ccx launches the owner with a warning.

A banner names the owner, target and reason, for example `ccx: handoff main → angel2 (main limited: 5h 92%)`, and the new id of a fork. The banner and warnings show only when stdout and stderr are terminals, not for `-p`.

No handoff occurs for:

- A bare `ccx --resume` (no id, or a search term). It opens the Claude Code session picker, so the session is unknown. ccx picks the account with automatic selection and passes `--resume` unchanged.
- `ccx @name -c` and `CCX_ACCOUNT=name ccx -c`. They use only that account.

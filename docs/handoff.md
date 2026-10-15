# Move a session to a different account

When one account reaches its limit, `ccx handoff` copies a session to a different account and resumes it there.

```sh
ccx handoff                       # List the threads for this directory from all accounts. On a terminal, select a thread and the target account.
ccx handoff --all                 # List the threads from all directories.
ccx handoff <id> @angel2          # Copy session <id> into angel2, then run `claude --resume <id>` in angel2. There are no prompts.
ccx handoff <id> @angel2 --no-resume   # Copy only. Print the session id in the target account.
```

The list shows the session id, the account, the time of the last change and the first prompt. When stdin or stderr is not a terminal, `ccx handoff` prints the list and stops. For `<id>`, you can give a unique part of 8 or more characters at the start of the id.

Handoff copies these files of the session from the source account into the target account:

- `projects/<dir>/<id>.jsonl` (the transcript). `<dir>` is the encoded name of the session directory.
- Each `projects/*/<id>/` directory (the sidecar directories).
- `file-history/<id>/` and `session-env/<id>/`, if they are present.
- `plans/<slug>.md` for each `slug` in the transcript, if the file is present.

Handoff does not copy `todos/`, `projects/<dir>/memory/` or `jobs/`. Handoff never moves, deletes or changes a file in the source account.

Handoff refuses when the session is open:

- A `sessions/<pid>.json` file in the source or the target account has the session id, the process is alive, and its start time agrees with `procStart`. Handoff never reads the `sessions/*.key` files.
- The transcript changed in the last 5 seconds.

When the target has a copy of the session already:

- If the copy in the target is a prefix of the source transcript, handoff overwrites it.
- If not, handoff does not change the copy in the target. It writes a fork with a new session id, and shows the new id. Refer to [ADR 0009](adr/0009-handoff-fork-by-rewrite.md).

When `oauthAccount.organizationUuid` in `.claude.json` is different in the two accounts, handoff shows a warning before the copy: Claude cannot use the earlier thinking in the target account, and the next responses can be slower and use more tokens.

After the copy, ccx runs `claude --resume <id>` in the target account from the directory of the session. It uses the same environment rules as `ccx @name`.

## Automatic handoff on `ccx -c` and `ccx --resume <id>`

`ccx -c`, `ccx --continue` and `ccx --resume <id>` (also `-r <id>` and `--resume=<id>`) find the account that owns the session from the files on disk. ccx also finds sessions that you started with plain `claude` or with an alias such as `claude2`.

- For `--resume <id>`, the owner is the account that has `projects/*/<id>.jsonl`. When more than one account has the session, the newest copy wins.
- For `-c`, the owner is the account with the newest `*.jsonl` in `projects/<encoded cwd>/`. When no account has a session for the current directory, ccx uses automatic selection.

Then:

1. If the owner is not limited, ccx launches the owner. It copies no files.
2. If the owner is limited (the same rules as automatic selection), ccx picks a target account with automatic selection. It copies the session with the handoff rules above and runs `claude --resume <id>` in the target. ccx changes `-c` to `--resume <id>`. If the handoff writes a fork, ccx resumes the fork.
3. If the owner is an [API-key account](api-key-accounts.md) and a subscription account is not limited, ccx hands off to the subscription account that automatic selection picks. It does not ask.
4. If all subscription accounts are limited, ccx asks `All subscription accounts are limited. Use paid account NAME? [y/N]`. NAME is the owner when the owner is an API-key account, else the first API-key account. With `--allow-paid`, ccx does not ask. If you answer yes, ccx resumes on the paid account (with a handoff when the owner is a subscription account). If you answer no, or there is no terminal and no `--allow-paid`, the session stays in a subscription owner and ccx shows a warning. A session of an API-key owner goes to the subscription account with the soonest reset.
5. If the handoff refuses, for example because the session is open, ccx launches the owner and shows a warning.

On a terminal, ccx shows a banner that names the owner, the target and the reason, for example `ccx: handoff main → angel2 (main limited: 5h 92%)`. For a fork, the banner also shows the new session id. ccx shows the banner and the warnings only when stdout and stderr are terminals, and not for `-p`.

A bare `ccx --resume` (with no id, or with a search term) opens the session picker of Claude Code. ccx cannot know which session you will select. Thus ccx does no handoff. It picks the account with automatic selection and passes `--resume` with no change.

`ccx @name -c` and `CCX_ACCOUNT=name ccx -c` use only that account. They never do a handoff.

# 0006. M0 test 4: session file layout and the cwd encoding

- **Status:** Accepted
- **Date:** 2026-10-12
- **Claude Code version:** 2.1.296
- **Platform:** macOS (Darwin 25)
- **Issue:** #2, test 4

## Context

`ccx handoff` (#14) copies a session from one account to a different account. Automatic handoff (#15) must know if a session is open. Thus, ccx must know:

- which files and directories belong to one session,
- how Claude Code makes the `projects/` directory name from the cwd,
- which file shows that a session is open.

Test 4 examined the two real accounts with read-only commands. The test read only the names of files and directories, the `slug` and `cwd` fields of transcripts, and the `sessions/<pid>.json` files. The test did not read the `*.key` files. To find the encoding rule, the test also ran `/usage` with a temporary, empty `CLAUDE_CONFIG_DIR` from directories with special names.

All paths in this ADR are examples. `/Users/example` replaces the real home directory.

## Results

### Files for one session

`<id>` is the session id (a UUID). `<enc>` is the encoded cwd (refer to the next section).

| Path in the account directory | Kind | Per session | Notes |
|---|---|---|---|
| `projects/<enc>/<id>.jsonl` | file | yes | The transcript. |
| `projects/<enc>/<id>/` | directory | yes | Sidecar directory. Contains `subagents/` (`agent-<hex>.jsonl`, `agent-<hex>.meta.json`, and sometimes `agent-<hex>.forked-skill.json` and `.forked-skill.marker.json`), `tool-results/` and `workflows/`. Not all sessions have it. |
| `projects/<other enc>/<id>/` | directory | yes | **A sidecar directory can also be in a different `projects/` directory**, for example when a workflow runs in a subdirectory. The test found a session with its transcript in `projects/-Users-example-projects/` and a second sidecar in `projects/-Users-example-projects-ccx/`. |
| `file-history/<id>/` | directory | yes | File backups, named `<16 hex>@v<N>`. |
| `session-env/<id>/` | directory | yes | The base account had 180 of these directories and 152 transcripts. Some directories have no transcript. **All 180 directories were empty.** |
| `plans/<slug>.md` | file | yes | Named with the `slug` field of the transcript, not with the session id. Example: `plans/okay-let-s-plan-out-staged-beacon.md`. 11 distinct slugs were in the transcripts, but only 2 plan files were present. Thus, a slug does not always have a plan file. One transcript had more than one slug. |
| `projects/<enc>/memory/` | directory | **no** | Project memory. It belongs to the cwd, not to a session. Do not copy it. |
| `todos/` | directory | — | **Not present** in either account. Current versions do not write `todos/`. |
| `jobs/<8 hex>/` | directory | no | Background jobs. Some names agree with the first 8 characters of a session id. Out of scope for handoff. |
| `shell-snapshots/`, `paste-cache/`, `telemetry/` | | no | Not keyed by session id. |

The `projects/<enc>/` directory contains only `<id>.jsonl` files, `<id>/` directories and `memory/`.

### The cwd encoding rule

Claude Code makes `<enc>` from the cwd as follows:

1. Use the physical path. Claude Code resolves symlinks first. A cwd that is a symlink to `/x/realdir` gives the name for `/x/realdir`.
2. Replace each UTF-16 code unit that is not `A-Z`, `a-z` or `0-9` with `-`. A `/`, `.`, `_`, space, `+`, `~`, `@` and `é` each give one `-`. An emoji outside the Basic Multilingual Plane (two UTF-16 code units) gives two `-`. There is no merge of `-` characters.
3. If the result has 200 characters or fewer, it is the name.
4. If not, the name is the first 200 characters, then `-`, then a hash of the **original** path in base 36. The hash is the Java string hash: `h = (31 * h + unit)` with 32-bit signed overflow over the UTF-16 code units, then the absolute value, then base 36 with lowercase letters.

The rule agreed with all 33 `projects/` directories that have a transcript with a `cwd` field, in the two accounts (33 of 33).

Golden examples for a test:

| cwd | `<enc>` |
|---|---|
| `/Users/example/projects/ccx` | `-Users-example-projects-ccx` |
| `/Users/example/.agents/mcp-servers/inkwire` | `-Users-example--agents-mcp-servers-inkwire` |
| `/private/tmp/a_b.c d+é~x@y` | `-private-tmp-a-b-c-d---x-y` |
| `/Users/example/emo😀j` | `-Users-example-emo--j` |
| `/Users/example/` followed by 200 `x` | `-Users-example-` followed by 185 `x`, then `-6k8g1n` (207 characters) |

### `sessions/<pid>.json`

Each open interactive session has one file `sessions/<pid>.json`. A file `sessions/<pid>.<64 hex>.key` is next to it. ccx must never read the `.key` file.

Fields (types only; the values below are synthetic):

```json
{
  "pid": 12345,
  "sessionId": "00000000-0000-4000-8000-000000000000",
  "cwd": "/Users/example/projects/ccx",
  "startedAt": 1767225600000,
  "procStart": "Thu Jan  1 00:00:00 2026",
  "version": "2.1.296",
  "kind": "interactive",
  "entrypoint": "cli",
  "pidDomain": "darwin",
  "status": "idle",
  "updatedAt": 1767225600000,
  "statusUpdatedAt": 1767225600000,
  "name": "projects-1",
  "nameSource": "derived",
  "nameSince": 1767225600000,
  "peerProtocol": 1,
  "peerFeatures": ["notify_idle"],
  "messagingSocketPath": "/tmp/cc-socket-example",
  "bridgeSessionId": "session_example",
  "waitingFor": "input needed"
}
```

- `waitingFor` is present only when `status` is `waiting`. Values seen for `status`: `busy`, `idle`, `waiting`.
- `procStart` is the process start time in the `ps -o lstart=` format, in **UTC**.
- All 4 files in the base account belonged to live processes. The angel2 account had no files. A print-mode (`-p`) run made no file.

## Decision

- **Files for handoff (#14):** for session `<id>`, copy:
  1. `projects/<enc>/<id>.jsonl`,
  2. each directory `projects/*/<id>/` (all `projects/` directories, not only `<enc>`), to the same relative path,
  3. `file-history/<id>/`,
  4. `session-env/<id>/`, if it is present,
  5. `plans/<slug>.md` for each distinct `slug` in the transcript, if the file is present.
- Do not copy `projects/<enc>/memory/`, `jobs/` or `todos/`.
- **Finding a session:** search `projects/*/<id>.jsonl`. Use the encoding rule only to find the sessions for the current cwd.
- **Open session (#14, #15):** a session is open when a file `sessions/<pid>.json` has `sessionId == <id>`, the process `pid` is alive, and the start time of that process agrees with `procStart`. The `procStart` check prevents a false result when the system uses the pid again. If ccx cannot get the start time of the process, it uses only the `pid` check.

## Consequences

- No D4 fallback is necessary. Test 4 had no adverse result.
- #14: the file set is different from the first plan. Remove `todos/`. Add the `plans/` slug rule and the search of all `projects/*/<id>/` directories.
- #14: implement the encoding rule as a pure function with the golden examples above as a unit test.
- #15: the open-session check uses `pid` and `procStart`. On Linux, the `procStart` format is not verified. Test it when Linux support starts.

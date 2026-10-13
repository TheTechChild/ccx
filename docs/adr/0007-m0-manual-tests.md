# 0007. M0 tests 2, 3, 5 and 6: results of the manual tests

- **Status:** Accepted
- **Date:** 2026-10-12
- **Claude Code version:** 2.1.296
- **Platform:** macOS (Darwin 25)
- **Issue:** #2, tests 2, 3, 5 and 6

## Context

Clayton ran these tests on the real accounts with the scripts in `scripts/m0/`. Refer to `scripts/m0/README.md` for the procedure.

## Results

| Test | Subject | Result |
|---|---|---|
| 2 | The `settings.json` symlink stays a symlink after `/config` writes in a linked account. | PASS |
| 3 | `/usage` writes `.claude.json` while a session in the same account is open. | PASS |
| 5 | `claude --resume <id>` works in account B after a copy of the session from account A. | PASS, with a caveat |
| 6 | A launchd job with a minimal `PATH` gets the keychain login. | PASS, with a caveat |

### Test 5 caveat: thinking does not move between organizations

When account B is in a different organization from account A, Claude Code resumes the session but shows this notice:

> Claude can't use thinking from another organization. Claude's earlier thinking was written while you were signed in to another organization, so Claude can't use it now. It isn't deleted: resume this session signed in to that organization to use it again. Claude's next responses may be slower and use more tokens because it has to reread this session and generate new thinking.

The transcript stays complete. Only the earlier thinking blocks are not usable in account B.

### Test 6 caveat: the service must use the absolute path to `claude`

The launchd job has `PATH=/usr/bin:/bin:/usr/sbin:/sbin`. In this environment:

| Case | Exit code | Usage limits in the output |
|---|---|---|
| A: bare `claude` | 127 (`env: claude: No such file or directory`) | no |
| B: absolute path to `claude`, base account | 0 | yes |
| C: absolute path to `claude`, second account | 0 | yes |

The keychain login works for the two accounts when the job uses the absolute path.

## Decision

- The approved fallbacks for tests 2, 5 and 6 are not necessary.
- **#13:** `ccx manage service install` writes the absolute path to `claude` in the launchd plist and the systemd unit. It finds the path when it installs the service. `ccx manage service status` and `ccx manage doctor` show an error when that path does not exist (for example, after a mise upgrade moves `claude`). They tell the user to run `ccx manage service install` again.
- **#14:** when the source and the target accounts are in different organizations, `ccx handoff` shows a warning before the copy: Claude cannot use the earlier thinking, and the next responses can be slower and use more tokens. ccx compares `oauthAccount.organizationUuid` in the `.claude.json` of the two accounts. If ccx cannot find the two values, it shows no warning.
- **#15:** the automatic handoff shows the same warning on a terminal.

## Consequences

- Handoff between organizations costs more tokens. Users can see this before the copy.
- The service needs a new install when the `claude` path changes. `doctor` finds this problem.

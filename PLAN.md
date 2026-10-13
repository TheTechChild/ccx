# ccx plan

ccx is a Rust command-line tool. It starts Claude Code on the least-used of several accounts. This file gives the issue order, the design decisions and the workflow rules.

## Issue order

Do the issues in this order. The GitHub issue is the specification for each step.

| Step | Issue | Title |
|---|---|---|
| 1 | #1 | Scaffold: mise.toml, Cargo project, CI on macOS and Linux, PLAN.md, ADRs |
| 2 | #2 | M0 tests: verify the 6 open assumptions and record the results in ADRs |
| 3 | #3 | `ccx @name [args]`: config load, canonical paths, environment stripping, `CCX_ACCOUNT`/`CCX_ACTIVE`, exec `claude` |
| 4 | #4 | Route `claude` subcommands to `@name` or to the base account |
| 5 | #5 | `ccx manage adopt` and `list` |
| 6 | #6 | `ccx manage shell-init`: completions and generated aliases |
| — | — | **Planned stop 1** |
| 7 | #7 | `ccx manage sync`: link all |
| 8 | #8 | `ccx manage doctor` |
| 9 | #9 | `ccx status` and the `/usage` fetch |
| 10 | #10 | Automatic selection |
| 11 | #11 | Stale-while-revalidate refresh |
| 12 | #12 | First release (`v0.1.0`) — **planned stop 2** |
| 13 | #13 | `ccx manage service install/uninstall/status` |
| 14 | #14 | `ccx handoff` |
| 15 | #15 | Automatic handoff on `ccx -c` and `--resume` |
| 16 | #16 | `ccx manage add` |
| 17 | #17 | Copy user MCP servers and preference keys |
| 18 | #18 | `ccx manage login --all`, `logout`, `edit`, `rm --purge` |
| 19 | #19 | `ccx manage bootstrap` |
| 20 | #20 | API-key accounts as paid fallback |

Follow-up issues: #21 (TUI wizards), #22 (adaptive refresh interval), #23 (direct usage API, policy review), #24 (monthly spend report).

## Architecture decisions

- [ADR 0001](docs/adr/0001-one-account-per-config-directory.md): one account for each configuration directory. `CLAUDE_CONFIG_DIR` is not set for the base account `~/.claude`.
- [ADR 0002](docs/adr/0002-no-credential-access.md): ccx never reads, keeps or refreshes Claude Code credentials. All logins go through `claude auth login`.
- [ADR 0003](docs/adr/0003-link-all-sharing.md): the sharing model is "link all".
- [ADR 0004](docs/adr/0004-usage-source.md): the usage source is `claude -p "/usage" --output-format stream-json`, with `cachedUsageUtilization` as the fallback.
- [ADR 0005](docs/adr/0005-m0-usage-hooks-off.md): M0 test 1. Hooks stay off for the `/usage` fetch.
- [ADR 0006](docs/adr/0006-m0-session-file-layout.md): M0 test 4. The session file layout and the cwd encoding rule.
- [ADR 0007](docs/adr/0007-m0-manual-tests.md): M0 tests 2, 3, 5 and 6. All pass. The caveats change #13, #14 and #15.
- [ADR 0008](docs/adr/0008-sync-share-lists.md): the share list, the never-share list and the shared `plugins/synced` risk of `ccx manage sync`.

## Decisions from the grilling session

- **Language and tools:** Rust, with `rust` and `cargo-dist` from mise. Users install ccx with `mise use -g github:TheTechChild/ccx`.
- **Base account:** the bare `claude` command stays on the base account. ccx makes aliases such as `claude2='ccx @angel2'`.
- **Accounts:** ccx supports many accounts of all kinds. ccx gets the kind and the tier from `claude auth status` and from `.claude.json`. If the tier is unknown, ccx shows the raw value.
- **Directories:** each account can be used in each directory. There are no pins.
- **Selection:** ccx selects by remaining capacity multiplied by a weight (Pro 1, Max 5x 5, Max 20x 20, Team and Enterprise 5; the configuration can change the weights). ccx stays on the last account if the difference is 10 points or less.
- **Limits:** an account is "limited" at session 90% or more, weekly 95% or more, or scoped weekly 98% or more. A scoped weekly limit applies only when the session model matches the scope name. Unknown usage counts as 50% used.
- **All accounts limited:** ccx starts the account with the soonest reset and shows a warning.
- **Usage refresh:** ccx uses a cache that is less than 15 minutes old and starts a detached background refresh before it starts `claude`. If the cache is older than 15 minutes, ccx fetches usage and waits for a maximum of 5 seconds. An optional service refreshes every 5 minutes. `ccx @name` fetches nothing.
- **Handoff:** ccx copies a session to a different account. It never moves the session. `ccx -c` and `ccx --resume <id>` do a handoff automatically when the owner account is limited.
- **Configuration:** `config.toml` has no email addresses. Email addresses go only in `~/.config/ccx/config.local.toml`, which ccx merges over `config.toml`.
- **New machines:** `ccx manage bootstrap` sets up a new machine. On a machine with no browser, ccx shows the login URL that `claude` prints.
- **API-key accounts:** these accounts are a paid fallback. The key is in the OS secret store. ccx asks before it uses a paid account, and refuses with no terminal unless you give `--allow-paid`.
- **Prompts:** ccx uses flags and yes/no prompts. It shows banners and warnings only on a terminal.

## Workflow rules

1. **Branches and pull requests:** use one branch and one pull request for each issue. Name the branch `issue-N-<slug>`. The pull request body says `Closes #N`.
2. **Commits:** use conventional commits. Write one sentence for each file in the commit message body.
3. **Author:** the commit author is `Clayton Noyes <20774670+TheTechChild@users.noreply.github.com>`.
4. **No attribution lines:** commits and pull requests have no attribution lines. Do not add `Co-Authored-By`, "Generated with" or session links.
5. **Merge:** merge the pull request when CI is green. Then start the next issue.
6. **Privacy rule:** do not put a real email address in the repository, an issue, a pull request, a fixture or a commit. Email addresses go only in `~/.config/ccx/config.local.toml`, which is never committed. Tests and docs use `@example.com` placeholders only. The `check-emails` task enforces this rule in CI.
7. **Real accounts (D3):** during development, Claude uses only read-only commands and `/usage` fetches on the real accounts. Claude never runs sync, adopt, handoff, login, logout or rm on a real account without asking Clayton. Tests use a temporary `HOME` and `testdata/fakebin/claude`.

## M0 results that change later issues

All six tests are done (claude 2.1.296). All pass. No approved fallback is necessary.

- **#9:** use the `/usage` command of ADR 0005 with `--settings '{"disableAllHooks":true}'`. Read `usage_report` from the top level of the `assistant` event, not from `message`. Accept `resets_at: null`. The fetch makes an empty `projects/<encoded cwd>/memory/` directory, so always run it from one fixed probe directory.
- **#14:** the handoff file set is `projects/<enc>/<id>.jsonl`, every `projects/*/<id>/` directory (a sidecar can be in a different `projects/` directory), `file-history/<id>/`, `session-env/<id>/` (empty in all cases seen) and `plans/<slug>.md` for each `slug` in the transcript. Do not copy `todos/` (not written), `memory/` or `jobs/`. Implement the cwd encoding rule of ADR 0006 with its golden examples.
- **#14, #15:** a session is open when `sessions/<pid>.json` has the `sessionId`, the `pid` is alive and its start time agrees with `procStart` (UTC, `ps -o lstart=` format). Never read `sessions/*.key`.
- **#13:** the service writes the absolute path to `claude`. `service status` and `doctor` show an error when that path does not exist (ADR 0007).
- **#14, #15:** when `oauthAccount.organizationUuid` is different in the two accounts, show a warning before the handoff: Claude cannot use the earlier thinking, and the next responses can use more tokens (ADR 0007).

## Planned stops

1. **After #6:** Clayton runs M0 tests 2, 3, 5 and 6. Done: all pass.
2. **At #12:** Clayton pushes the `v0.1.0` tag and does the Linux install check.
3. **Any M0 result that has no approved fallback:** stop and ask Clayton.

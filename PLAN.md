# ccx history and status

ccx is a Rust command-line tool. It starts Claude Code on the least-used of several accounts. This file gives the project history, the status, the design decisions and the M0 results.

For the contributor rules, refer to [CONTRIBUTING.md](CONTRIBUTING.md) and [AGENTS.md](AGENTS.md).

## Status

The current version is 0.1.3. All planned issues are done. New work starts with a new GitHub issue.

## Releases

| Release | Contents |
|---|---|
| v0.1.0 | The first release: #1-#11 and #13-#24, made with the release process of #12. |
| v0.1.1 | Sync never shares `chrome`, `daemon.log`, `downloads` and `feedback`. The config error tells the user to run `ccx manage adopt` when `config.toml` does not exist. |
| v0.1.2 | #55: doctor `INFO` for unknown items and orphan keychain items, and sync asks which unknown items to share. |
| v0.1.3 | ADR 0013: `ccx @name` warns about a limited account, running sessions share the score of an account, and a usage-limit hit at the end of a transcript limits the account. Docs: contributor docs (#58), and the reference text of each command moves from the README to `docs/`. |

## Issues

The GitHub issue is the specification for each step. All issues in this table are done.

| Step | Issue | Title | Release |
|---|---|---|---|
| 1 | #1 | Scaffold: mise.toml, Cargo project, CI on macOS and Linux, PLAN.md, ADRs | v0.1.0 |
| 2 | #2 | M0 tests: verify the 6 open assumptions and record the results in ADRs | v0.1.0 |
| 3 | #3 | `ccx @name [args]`: config load, canonical paths, environment stripping, `CCX_ACCOUNT`/`CCX_ACTIVE`, exec `claude` | v0.1.0 |
| 4 | #4 | Route `claude` subcommands to `@name` or to the base account | v0.1.0 |
| 5 | #5 | `ccx manage adopt` and `list` | v0.1.0 |
| 6 | #6 | `ccx manage shell-init`: completions and generated aliases | v0.1.0 |
| 7 | #7 | `ccx manage sync`: link all | v0.1.0 |
| 8 | #8 | `ccx manage doctor` | v0.1.0 |
| 9 | #9 | `ccx status` and the `/usage` fetch | v0.1.0 |
| 10 | #10 | Automatic selection | v0.1.0 |
| 11 | #11 | Stale-while-revalidate refresh | v0.1.0 |
| 12 | #12 | First release (`v0.1.0`) with cargo-dist | v0.1.0 |
| 13 | #13 | `ccx manage service install/uninstall/status` | v0.1.0 |
| 14 | #14 | `ccx handoff` | v0.1.0 |
| 15 | #15 | Automatic handoff on `ccx -c` and `--resume` | v0.1.0 |
| 16 | #16 | `ccx manage add` | v0.1.0 |
| 17 | #17 | Copy user MCP servers and preference keys | v0.1.0 |
| 18 | #18 | `ccx manage login --all`, `logout`, `edit`, `rm --purge` | v0.1.0 |
| 19 | #19 | `ccx manage bootstrap` | v0.1.0 |
| 20 | #20 | API-key accounts as paid fallback | v0.1.0 |
| 21 | #21 | Follow-up: TUI wizards for the manage commands (ADR 0011) | v0.1.0 |
| 22 | #22 | Follow-up: adaptive refresh interval | v0.1.0 |
| 23 | #23 | Follow-up: direct usage API, policy review. Result: no-go (ADR 0012) | v0.1.0 |
| 24 | #24 | Follow-up: monthly spend report for API-key accounts | v0.1.0 |
| 25 | #55 | Doctor `INFO` for unknown items and orphan keychain items; sync asks which unknown items to share | v0.1.2 |
| 26 | #58 | Contributor docs: `AGENTS.md`, `CLAUDE.md`, `CONTRIBUTING.md`, README and PLAN.md updates | v0.1.3 |
| 27 | - | Session limits: named-launch warning, running sessions in the score, limit hits from transcripts (ADR 0013) | v0.1.3 |

## Architecture decisions

- [ADR 0001](docs/adr/0001-one-account-per-config-directory.md): one account for each configuration directory. `CLAUDE_CONFIG_DIR` is not set for the base account `~/.claude`.
- [ADR 0002](docs/adr/0002-no-credential-access.md): ccx never reads, keeps or refreshes Claude Code credentials. All logins go through `claude auth login`.
- [ADR 0003](docs/adr/0003-link-all-sharing.md): the sharing model is "link all".
- [ADR 0004](docs/adr/0004-usage-source.md): the usage source is `claude -p "/usage" --output-format stream-json`, with `cachedUsageUtilization` as the fallback.
- [ADR 0005](docs/adr/0005-m0-usage-hooks-off.md): M0 test 1. Hooks stay off for the `/usage` fetch.
- [ADR 0006](docs/adr/0006-m0-session-file-layout.md): M0 test 4. The session file layout and the cwd encoding rule.
- [ADR 0007](docs/adr/0007-m0-manual-tests.md): M0 tests 2, 3, 5 and 6. All pass. The caveats change #13, #14 and #15.
- [ADR 0008](docs/adr/0008-sync-share-lists.md): the share list, the never-share list and the shared `plugins/synced` risk of `ccx manage sync`.
- [ADR 0009](docs/adr/0009-handoff-fork-by-rewrite.md): `ccx handoff` makes a fork with a new session id itself. It does not use `--fork-session`.
- [ADR 0010](docs/adr/0010-api-key-store.md): API-key accounts keep their keys through the OS commands `security` and `secret-tool`, not the `keyring` crate.
- [ADR 0011](docs/adr/0011-wizards-with-dialoguer.md): the `ccx manage` wizards use multi-step `dialoguer` prompts (decision D6). ccx adds no TUI crate.
- [ADR 0012](docs/adr/0012-no-direct-usage-api.md): ccx does not call `/api/oauth/usage` directly (decision D6). That call needs the OAuth token, and ccx never reads credentials (ADR 0002).
- [ADR 0013](docs/adr/0013-session-limit-awareness.md): `ccx @name` warns about a limited account, running sessions share the score of an account, and a usage-limit hit at the end of a transcript limits the account until the reset.

## Design decisions

These decisions come from the design review before the first build.

- **Language and tools:** Rust, with `rust` and `cargo-dist` from mise. Users install ccx with `mise use -g github:TheTechChild/ccx`.
- **Base account:** the bare `claude` command stays on the base account. ccx makes aliases such as `claude2='ccx @angel2'`.
- **Accounts:** ccx supports many accounts of all kinds. ccx gets the kind and the tier from `claude auth status` and from `.claude.json`. If the tier is unknown, ccx shows the raw value.
- **Directories:** each account can be used in each directory. There are no pins.
- **Selection:** ccx selects by remaining capacity multiplied by a weight (Pro 1, Max 5x 5, Max 20x 20, Team and Enterprise 5; the configuration can change the weights). ccx stays on the last account if the difference is 10 points or less.
- **Limits:** an account is "limited" at session 90% or more, weekly 95% or more, or scoped weekly 98% or more. A scoped weekly limit applies only when the session model matches the scope name. Unknown usage counts as 50% used.
- **All accounts limited:** ccx starts the account with the soonest reset and shows a warning.
- **Usage refresh:** ccx uses a cache that is less than 15 minutes old and starts a detached background refresh before it starts `claude`. If the cache is older than 15 minutes, ccx fetches usage and waits for a maximum of 5 seconds. An optional service refreshes the accounts that are due by the adaptive interval (#22). `ccx @name` fetches nothing.
- **Handoff:** ccx copies a session to a different account. It never moves the session. `ccx -c` and `ccx --resume <id>` do a handoff automatically when the owner account is limited.
- **Configuration:** `config.toml` has no email addresses. Email addresses go only in `~/.config/ccx/config.local.toml`, which ccx merges over `config.toml`.
- **New machines:** `ccx manage bootstrap` sets up a new machine. On a machine with no browser, ccx shows the login URL that `claude` prints.
- **API-key accounts:** these accounts are a paid fallback. The key is in the OS secret store. ccx asks before it uses a paid account, and refuses with no terminal unless you give `--allow-paid`.
- **Prompts:** ccx uses flags and yes/no prompts. On a terminal, `manage add`, `edit` and `rm` with no NAME, and `manage adopt` with no `-y`, start a wizard (#21, ADR 0011). It shows banners and warnings only on a terminal.
- **Real accounts:** development uses only read-only commands and `/usage` fetches on real accounts. Tests use a temporary `HOME` and `testdata/fakebin/claude`. The rules are in [AGENTS.md](AGENTS.md).

## M0 results

All six M0 tests are done (claude 2.1.296). All pass. No fallback was necessary. These results changed the issues below, and the code follows them.

- **#9:** use the `/usage` command of ADR 0005 with `--settings '{"disableAllHooks":true}'`. Read `usage_report` from the top level of the `assistant` event, not from `message`. Accept `resets_at: null`. The fetch makes an empty `projects/<encoded cwd>/memory/` directory, so always run it from one fixed probe directory.
- **#14:** the handoff file set is `projects/<enc>/<id>.jsonl`, every `projects/*/<id>/` directory (a sidecar can be in a different `projects/` directory), `file-history/<id>/`, `session-env/<id>/` (empty in all cases seen) and `plans/<slug>.md` for each `slug` in the transcript. Do not copy `todos/` (not written), `memory/` or `jobs/`. Implement the cwd encoding rule of ADR 0006 with its golden examples.
- **#14, #15:** a session is open when `sessions/<pid>.json` has the `sessionId`, the `pid` is alive and its start time agrees with `procStart` (UTC, `ps -o lstart=` format). Never read `sessions/*.key`.
- **#13:** the service writes the absolute path to `claude`. `service status` and `doctor` show an error when that path does not exist (ADR 0007).
- **#14, #15:** when `oauthAccount.organizationUuid` is different in the two accounts, show a warning before the handoff: Claude cannot use the earlier thinking, and the next responses can use more tokens (ADR 0007).

## History of the first build

The first build (#1-#24) used these rules. They do not apply to new work. The current rules are in [AGENTS.md](AGENTS.md) and [CONTRIBUTING.md](CONTRIBUTING.md).

- The issues were done in the order of the table above, one pull request for each issue.
- There were planned stops: after #6 for the manual M0 tests 2, 3, 5 and 6, at #12 for the `v0.1.0` tag and the Linux install check, and for each M0 result with no approved fallback. The M0 tests passed, and no stop for a fallback was necessary.
- Decision D3 limited the actions on the real accounts of the maintainer to read-only commands and `/usage` fetches. [AGENTS.md](AGENTS.md) keeps this rule for all contributors.

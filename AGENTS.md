# AGENTS.md

These are the instructions for coding agents that work on ccx. `CLAUDE.md` imports this file. Keep all agent instructions here.

## What ccx is

ccx is a Rust command-line tool. It starts Claude Code (`claude`) on the account that has the most usage available, or on the account that you name with `@name`. Each account is one Claude Code configuration directory. ccx also manages the accounts: adopt, add, sync, login, handoff of sessions, and a background usage refresh.

[README.md](README.md) tells new users how to install and start ccx. The files in [`docs/`](docs/) tell how each command works. [PLAN.md](PLAN.md) has the project history and the design decisions.

## Commands

All tools come from [mise](https://mise.jdx.dev). Do not install Rust or other tools in a different way. Run cargo through mise, for example `mise exec -- cargo build`.

| Command | What it does |
|---|---|
| `mise install` | Install the tools in `mise.toml` (Rust with rustfmt and clippy, and cargo-dist). |
| `mise run ci` | Run all checks: `fmt`, `lint`, `test` and `check-emails`. CI runs this command on macOS and Linux. |
| `mise run fmt` | `cargo fmt --all -- --check`. |
| `mise run lint` | `cargo clippy --all-targets --locked -- -D warnings`. |
| `mise run test` | `cargo test --locked`. |
| `mise run check-emails` | `scripts/check-emails.sh`: fail on each email address that is not `*@example.com` and does not contain `noreply`. |
| `CCX_UPDATE_GOLDEN=1 mise run test` | Write the golden files again (the completions in `completions/` and the files in `testdata/`). Examine the diff before you commit. |

`mise run ci` must pass before you push.

## Module map

The binary is `src/main.rs`. It has no library crate.

| File | Contents |
|---|---|
| `src/main.rs` | The clap CLI definition, the dispatch of the ccx commands and the launch flow (automatic selection, automatic handoff, `exec` of `claude`). |
| `src/launch.rs` | The launch path `ccx [@name] [claude args...]`: argument split, `CLAUDE_SUBCOMMANDS`, the environment of the account, `find_claude`. Keep this path fast. |
| `src/config.rs` | Load, merge and validate `config.toml` and the overlay `config.local.toml`. |
| `src/paths.rs` | Canonical account paths (`canonical`, `expand_tilde`, `tilde`). |
| `src/env.rs` | A copy of the process environment, so that tests never change the real environment. |
| `src/pick.rs` | Automatic selection: the pure picker, the pick state (`state.json`) and the banner. |
| `src/usage.rs` | The `/usage` fetch, its parser, the usage cache and the `cachedUsageUtilization` fallback. |
| `src/refresh.rs` | Stale-while-revalidate: the refresh lock, the 60-second rule, the blocking fetch and the detached background refresh. |
| `src/schedule.rs` | The adaptive interval of the scheduled refresh (`schedule.json`). |
| `src/service.rs` | `ccx manage service` (`install`, `uninstall`, `status`): launchd, systemd user timer or a crontab line. |
| `src/status.rs` | `ccx status` and `ccx usage refresh`. |
| `src/adopt.rs` | `ccx manage adopt`: find the accounts on this machine and write the config. |
| `src/add.rs` | `ccx manage add NAME`: make a new subscription account in one step. |
| `src/add_api_key.rs` | `ccx manage add NAME --api-key`: make an API-key account. |
| `src/keystore.rs` | The keys of API-key accounts: OS secret store items `ccx-<name>`, `key_command` or key files. |
| `src/paid.rs` | The consent rules of the paid fallback and the key in the launch environment. |
| `src/lifecycle.rs` | `ccx manage login`, `logout`, `edit`, `edit-config` and `rm`. |
| `src/bootstrap.rs` | `ccx manage bootstrap`: set up ccx on a new machine. |
| `src/list.rs` | `ccx manage list [--json]`. |
| `src/spend.rs` | `ccx manage spend`: the monthly spend estimate of API-key accounts. |
| `src/shell_init.rs` | `ccx manage shell-init` for zsh, bash and fish: completions and aliases. |
| `src/sync.rs` | `ccx manage sync`: share the base account through symlinks. It has the `SHARED` and `NEVER` lists. |
| `src/prefs.rs` | `ccx manage sync --prefs`: copy user MCP servers and allowlisted preference keys. |
| `src/doctor.rs` | `ccx manage doctor`: check the setup. It only reads. |
| `src/handoff.rs` | `ccx handoff`: copy a session to a different account and resume it there. |
| `src/resume.rs` | The automatic handoff on `ccx -c`, `ccx --continue` and `ccx --resume <id>`. |
| `src/wizard.rs` | The multi-step `dialoguer` prompts of the `ccx manage` commands. |

## How to test

- Unit tests are in a `#[cfg(test)]` module at the end of each file in `src/`. Integration tests are in `tests/`. They run the `ccx` binary (`env!("CARGO_BIN_EXE_ccx")`).
- Each integration test makes a temporary `HOME` with `tempfile::tempdir()`, writes `config.toml` and `config.local.toml` in it, and puts `testdata/fakebin` first on `PATH` (or sets `CCX_CLAUDE_BIN` to the fake `claude`).
- `testdata/fakebin/` has fake programs. Each file starts with a comment that tells its behavior and the `FAKE_*` variables that control it:
  - `claude`: prints its argv, cwd and environment as JSON. It also fakes `auth status`, `auth login`, `auth logout`, `-p /usage` and `mcp add-json`.
  - `security` and `secret-tool`: a fake OS secret store in `FAKE_SECRET_DIR`.
  - `launchctl`, `systemctl` and `loginctl`: fake service managers. They log each call to `FAKE_SERVICE_LOG`.
- Golden files: `testdata/doctor/`, `testdata/service/`, `testdata/shell-init/`, `testdata/spend/` and `completions/`. To write them again, run `CCX_UPDATE_GOLDEN=1 mise run test`.
- Wizard tests use a scripted prompt from `src/wizard.rs`, not a real terminal.
- Environment variables that tests use:

| Variable | Use |
|---|---|
| `CCX_UPDATE_GOLDEN` | Write the golden files again. |
| `CCX_TEST_SHELLS` | A list such as `zsh,bash,fish`. The shell-init tests fail when a shell in the list is not on `PATH`. Without it, a missing shell is skipped. |
| `CCX_NO_REFRESH` | Stop the blocking fetch and the background refresh of a launch. |
| `CCX_SERVICE_KIND` | Select the service manager: `launchd`, `systemd` or `cron`. |
| `CCX_CLAUDE_BIN` | The path of `claude`, for example the fake `claude`. |
| `CCX_CONFIG` | The path of `config.toml`. |
| `CCX_DRY_RUN` | Show the launch plan and do not start `claude`. |

- `tests/no_credentials.rs` is the guard test for ADR 0002. `tests/check_emails.rs` tests `scripts/check-emails.sh`.
- `scripts/m0/` has the manual M0 tests (refer to ADR 0005, ADR 0006 and ADR 0007). They are not part of `mise run ci`.

## Hard rules

1. **Credentials.** ccx never reads, stores or refreshes Claude Code credentials. Do not read `.credentials.json`, the keychain secrets of Claude Code or `sessions/*.key`. Do not call the OAuth token endpoint. All logins go through `claude auth login`. Refer to [ADR 0002](docs/adr/0002-no-credential-access.md) and [ADR 0012](docs/adr/0012-no-direct-usage-api.md). The only secrets that ccx reads are the keys of API-key accounts, in `src/keystore.rs` ([ADR 0010](docs/adr/0010-api-key-store.md)).
2. **Real accounts.** Tests never use the real `~/.claude`, `~/.claude-*`, `~/.claude.json`, the real keychain or a real service manager. Use a temporary `HOME` and the fakes in `testdata/fakebin/`. During development, run only read-only commands on real accounts. Do not run sync, adopt, add, handoff, login, logout, rm, bootstrap or service install on a real account.
3. **Privacy.** Do not put a real email address in a file, a fixture, a commit, an issue or a pull request. Use `*@example.com` addresses. `mise run check-emails` scans all tracked files and all commits.
4. **Canonical paths.** Claude Code keys an account by the exact text of its config directory. Use `paths::canonical` for account dirs: absolute, with no trailing slash, and with no symlink resolution. Do not set `CLAUDE_CONFIG_DIR` for the base account `~/.claude` ([ADR 0001](docs/adr/0001-one-account-per-config-directory.md)).
5. **Never-share list.** Sync never shares the items in `NEVER` in `src/sync.rs` or a dotfile. Do not remove an item from `NEVER` without a new ADR. When you change `SHARED` or `NEVER`, update [ADR 0008](docs/adr/0008-sync-share-lists.md) and [docs/sync.md](docs/sync.md).
6. **No data loss.** Sync, adopt and `rm --purge` move user files to a backup or a trash directory. Sync never deletes through a symlink: `RELINK` removes only the link. Handoff never changes a file of the source account.
7. **Fast launch.** The launch path must stay fast. Do not add network calls or slow work before `exec` of `claude`.
8. **Few dependencies.** Add a crate only when it is necessary. Write simple, idiomatic Rust with tests.

## Decisions

The architecture decisions are in [`docs/adr/`](docs/adr/). Read the ADRs before you change the behavior that they cover. When you make a new decision, add an ADR: copy the format of an existing ADR, give it the next number (for example `0013-<slug>.md`), and add a line for it in [PLAN.md](PLAN.md).

## Commits and pull requests

- Open an issue first. Use one branch for each issue, named `issue-N-<slug>`, from an up-to-date `main`.
- Use [conventional commits](https://www.conventionalcommits.org/), for example `feat(sync): ...` or `fix(config): ...`. In the commit message body, write one sentence for each file in the commit.
- Do not add attribution lines to commits or pull requests: no `Co-Authored-By`, no "Generated with", no session links. A pull request body ends with its last content line.
- Use one pull request for each issue, with base `main`. The body has `Closes #N` and the acceptance criteria of the issue as a checked list.
- CI must be green on macOS and Linux before merge.

## Documentation

Keep the README short: install, the quick start and links. Put the reference text for each command in its file in `docs/`. When the behavior changes, update that file.

Write the docs (README, `docs/`, ADRs, these files) and the user-facing text in ASD-STE100 Simplified Technical English: short sentences, one instruction in each sentence, the active voice, and simple words. Make sure that each command, file path, module name, environment variable and config key in the docs exists in the code.

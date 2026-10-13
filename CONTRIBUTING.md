# Contributing to ccx

Thank you for your help. This file tells you how to set up the project, run the checks and propose a change. [AGENTS.md](AGENTS.md) has the full details: the module map, the test method and the hard rules. Coding agents use the same file.

## Set up

ccx uses [mise](https://mise.jdx.dev) for all tools. Do not install Rust in a different way.

1. Install mise.
2. Clone the repository: `git clone https://github.com/TheTechChild/ccx.git`.
3. In the repository, install the tools: `mise install`.
4. Run cargo through mise, for example `mise exec -- cargo build`.

## Run the checks

```sh
mise run ci                          # All checks. CI runs this on macOS and Linux.
mise run fmt                         # The code format.
mise run lint                        # clippy, with warnings as errors.
mise run test                        # All tests.
mise run check-emails                # The privacy check.
CCX_UPDATE_GOLDEN=1 mise run test    # Write the golden files again. Examine the diff.
```

`mise run ci` must pass before you push.

## Tests and safety rules

- Tests never use your real accounts. Each test uses a temporary `HOME` and the fake programs in `testdata/fakebin/` (`claude`, `security`, `secret-tool`, `launchctl`, `systemctl` and `loginctl`).
- ccx never reads, stores or refreshes Claude Code credentials ([ADR 0002](docs/adr/0002-no-credential-access.md)). A test fails if the code reads `.credentials.json` or a keychain secret.
- Do not put a real email address in the repository, a commit, an issue or a pull request. Use `*@example.com` addresses. `mise run check-emails` fails on all other addresses.
- When you work on ccx, do not run sync, adopt, add, handoff, login, logout, rm, bootstrap or service install on your real accounts with a development build.

Refer to [AGENTS.md](AGENTS.md) for the full list of rules.

## Propose a change

1. Open an issue first. Tell what to change and why. For a feature, write the acceptance criteria.
2. Make a branch from an up-to-date `main`. Name it `issue-N-<slug>`, for example `issue-42-status-colors`.
3. Make the change with tests. Update the README and the ADRs when the behavior changes.
4. Use [conventional commits](https://www.conventionalcommits.org/), for example `fix(sync): ...`. In the commit message body, write one sentence for each file in the commit. Do not add attribution lines such as `Co-Authored-By`.
5. Open one pull request for each issue, with base `main`. In the body, write `Closes #N` and the acceptance criteria as a checked list.
6. CI must be green on macOS and Linux before merge.

Write the docs and the user-facing text in ASD-STE100 Simplified Technical English: short sentences, the active voice and simple words.

## Add an ADR

The architecture decisions are in [`docs/adr/`](docs/adr/). Add an ADR when you make a decision that changes the design, a safety rule or a list such as the sync share lists.

1. Copy the format of an existing ADR.
2. Give it the next number and a short name, for example `docs/adr/0013-<slug>.md`.
3. Write the context, the decision and the consequences.
4. Add a line for the ADR in [PLAN.md](PLAN.md).

## Make a release

The maintainer makes the releases with cargo-dist. Refer to [Make a release](README.md#make-a-release) in the README.

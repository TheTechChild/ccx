# ccx

ccx is a command-line tool. It starts Claude Code on the account that has the most usage available.

ccx is for a person who has more than one Claude Code account, for example a personal Max account and a work account. Each account has its own configuration directory. ccx reads the usage of each account and starts `claude` on the account that has the most capacity available. You can also name the account.

> **Status:** Version 0.1.2. `ccx @name`, automatic selection, `ccx status`, `ccx usage refresh`, `ccx manage adopt`, `ccx manage add`, `ccx manage bootstrap`, `ccx manage login`, `ccx manage logout`, `ccx manage edit`, `ccx manage edit-config`, `ccx manage rm`, `ccx manage list`, `ccx manage spend`, `ccx manage shell-init`, `ccx manage sync` (also `--prefs`), `ccx manage doctor`, `ccx manage service`, `ccx handoff`, the automatic handoff on `ccx -c` and `ccx --resume <id>`, and API-key accounts as a paid fallback are available. Refer to [PLAN.md](PLAN.md) for the project history.

## Credential policy

ccx never reads, stores or refreshes Claude Code credentials. ccx does not read `.credentials.json` or the keychain secrets of Claude Code. All logins go through `claude auth login`. ccx gets the usage data from the `claude` program. Refer to [ADR 0002](docs/adr/0002-no-credential-access.md).

The only secrets that ccx reads are the keys of [API-key accounts](docs/api-key-accounts.md). ccx stores these keys itself, as the OS secret store items `ccx-NAME` or in key files at mode 0600. Refer to [ADR 0010](docs/adr/0010-api-key-store.md).

## Install

Install ccx with [mise](https://mise.jdx.dev):

```sh
mise use -g github:TheTechChild/ccx@latest
ccx --version
```

To get a new release later, run `mise upgrade github:TheTechChild/ccx`.

mise does not show a release that is less than 24 hours old (the mise setting `minimum_release_age`). Thus, for 24 hours after a release, mise installs the version before it. To get a new release immediately, do one of these steps:

- Exclude ccx from the setting one time for all installs: `mise settings add minimum_release_age_excludes "github:TheTechChild/ccx"`.
- Ignore the setting for one command: `mise use -g github:TheTechChild/ccx@latest --minimum-release-age 0s` or `mise upgrade github:TheTechChild/ccx --minimum-release-age 0s`.

mise downloads the release archive for your system from GitHub and puts `ccx` on `PATH`. The releases are for macOS and Linux, on x86_64 (amd64) and arm64 (aarch64). Each release has a `sha256.sum` file with the checksums of the archives.

You must also have Claude Code (`claude`) on `PATH`.

Each archive also has the static completion files in `completions/`: `_ccx` (zsh), `ccx.bash` and `ccx.fish`. These files do not complete the account names. To get the account names and the aliases, use `ccx manage shell-init` (refer to [Add completions and aliases to your shell](docs/shell.md)).

## Quick start

1. Add the accounts that are on this machine already. These are the directories that you logged in to with `claude auth login`. The base account uses `~/.claude`. Each other account uses its own directory, for example `~/.claude-angel2`. Run `ccx manage adopt` to add them to the ccx config.
2. Add each new account: `ccx manage add NAME`. Add makes the directory `~/.claude-NAME`, links the shared setup of the base account, runs `claude auth login` and writes the config. Run `ccx manage add` with no NAME to start the wizard. Refer to [Add a new account](docs/accounts.md#add-a-new-account). For an Anthropic API key, refer to [Add an API-key account](docs/api-key-accounts.md#add-an-api-key-account).
3. Add completions and aliases to your shell profile: `eval "$(ccx manage shell-init zsh)"`. For example, `aliases = ["claude2"]` in account `angel2` gives `alias claude2='ccx @angel2'`. Refer to [Configuration](docs/configuration.md).
4. Start Claude Code:
   - `ccx` selects the account that has the most capacity available.
   - `ccx @angel2` starts account `angel2`.
5. Show the usage of each account: `ccx status`.

The bare `claude` command stays on the base account. ccx never makes an alias for `claude`.

On a new machine that has a `config.toml` already, use `ccx manage bootstrap` for all the setup steps (refer to [Set up a new machine](docs/accounts.md#set-up-a-new-machine)).

## Documentation

| Task | Command | Documentation |
|---|---|---|
| Start Claude Code, automatic selection, the environment | `ccx`, `ccx @name` | [docs/launch.md](docs/launch.md) |
| Set up a new machine, and adopt, add, log in, change, remove and list accounts | `ccx manage bootstrap`, `adopt`, `add`, `login`, `logout`, `edit`, `edit-config`, `rm`, `list` | [docs/accounts.md](docs/accounts.md) |
| Use an API key as a paid fallback, and show its spend | `ccx manage add NAME --api-key`, `ccx manage spend` | [docs/api-key-accounts.md](docs/api-key-accounts.md) |
| Move a session to a different account | `ccx handoff`, `ccx -c`, `ccx --resume <id>` | [docs/handoff.md](docs/handoff.md) |
| Share the setup of the base account | `ccx manage sync`, `ccx manage sync --prefs` | [docs/sync.md](docs/sync.md) |
| Show the usage, and keep it fresh | `ccx status`, `ccx usage refresh`, `ccx manage service` | [docs/usage-data.md](docs/usage-data.md) |
| Add completions and aliases to your shell | `ccx manage shell-init` | [docs/shell.md](docs/shell.md) |
| Check the setup | `ccx manage doctor` | [docs/doctor.md](docs/doctor.md) |
| Change `config.toml` and `config.local.toml` | `ccx manage edit-config` ([Change an account](docs/accounts.md#change-an-account)) | [docs/configuration.md](docs/configuration.md) |

The architecture decisions are in [docs/adr/](docs/adr/).

## How ccx works

- Each account has its own Claude Code configuration directory. The base account uses `~/.claude`.
- ccx sets `CLAUDE_CONFIG_DIR` for the account that it selects, and then starts `claude`.
- ccx does not read, keep or refresh Claude Code credentials. All logins go through `claude auth login`.

## Development

[CONTRIBUTING.md](CONTRIBUTING.md) tells you how to set up the project, run the checks, propose a change and make a release. [AGENTS.md](AGENTS.md) has the module map, the test method and the hard rules. `CLAUDE.md` imports `AGENTS.md` for Claude Code.

## Privacy rule

Do not put a real email address in this repository. Use `@example.com` addresses in code, tests and docs. The `check-emails` check fails on all other addresses, except addresses that contain `noreply`.

## License

MIT. Refer to [LICENSE](LICENSE).

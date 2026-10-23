# ccx

ccx is a command-line tool for more than one Claude Code account, such as a personal Max account and a work account. Each account has its own configuration directory. ccx reads the usage of each account. It uses the base account until it is limited, then the next account, or the account that you name. It sets `CLAUDE_CONFIG_DIR` for that account (not for the base account `~/.claude`) and starts `claude`.

> **Status:** Version 0.1.2. All the commands and features in the [Documentation](#documentation) table are available. Refer to [PLAN.md](PLAN.md) for the project history.

## Credential policy

- ccx never reads, stores or refreshes Claude Code credentials, such as `.credentials.json` or the keychain secrets of Claude Code ([ADR 0002](docs/adr/0002-no-credential-access.md)). All logins go through `claude auth login`. ccx gets usage data from `claude`.
- The only secrets that ccx reads are the keys of [API-key accounts](docs/api-key-accounts.md): the OS secret store items `ccx-NAME` or key files at mode 0600 ([ADR 0010](docs/adr/0010-api-key-store.md)).

## Install

You must have Claude Code (`claude`) on `PATH`. Install ccx with [mise](https://mise.jdx.dev):

```sh
mise use -g github:TheTechChild/ccx@latest
ccx --version
mise upgrade github:TheTechChild/ccx   # get a new release later
```

- mise downloads the release archive for your system from GitHub and puts `ccx` on `PATH`. Releases are for macOS and Linux, on x86_64 (amd64) and arm64 (aarch64). Each release has a `sha256.sum` file with the checksums of the archives.
- Each archive has static completions in `completions/`: `_ccx` (zsh), `ccx.bash` and `ccx.fish`. They do not complete account names. For account names and aliases, use [`ccx manage shell-init`](docs/shell.md).

mise does not show a release that is less than 24 hours old (the mise setting `minimum_release_age`). Until then, mise installs the previous release. To get a new release now:

- For all installs, exclude ccx from the setting one time: `mise settings add minimum_release_age_excludes "github:TheTechChild/ccx"`.
- For one command, add `--minimum-release-age 0s` to `mise use -g github:TheTechChild/ccx@latest` or `mise upgrade github:TheTechChild/ccx`.

## Quick start

1. Add the accounts on this machine that you logged in to with `claude auth login`: `ccx manage adopt`. The base account uses `~/.claude`. Each other account uses its own directory, for example `~/.claude-angel2`.
2. Add each new account: `ccx manage add NAME`. Add makes `~/.claude-NAME`, links the shared setup of the base account, runs `claude auth login` and writes the config. With no NAME, it starts the wizard. Refer to [Add a new account](docs/accounts.md#add-a-new-account). For an Anthropic API key, refer to [Add an API-key account](docs/api-key-accounts.md#add-an-api-key-account).
3. Add completions and aliases to your shell profile: `eval "$(ccx manage shell-init zsh)"`. For example, `aliases = ["claude2"]` in account `angel2` gives `alias claude2='ccx @angel2'`. Refer to [Configuration](docs/configuration.md).
4. Start Claude Code: `ccx` selects the first account that is not limited: the base account, then the other accounts in name order. `ccx @angel2` starts account `angel2`.
5. Show the usage of each account: `ccx status`.

The bare `claude` command stays on the base account. ccx never makes an alias for `claude`.

On a new machine with a `config.toml`, `ccx manage bootstrap` does all the setup steps. Refer to [Set up a new machine](docs/accounts.md#set-up-a-new-machine).

## Documentation

| Task | Command | Documentation |
|---|---|---|
| Start Claude Code, automatic selection, the environment | `ccx`, `ccx @name` | [docs/launch.md](docs/launch.md) |
| Set up a machine and manage accounts | `ccx manage bootstrap`, `adopt`, `add`, `login`, `logout`, `edit`, `edit-config`, `rm`, `list` | [docs/accounts.md](docs/accounts.md) |
| API-key accounts (paid fallback) and their spend | `ccx manage add NAME --api-key`, `ccx manage spend` | [docs/api-key-accounts.md](docs/api-key-accounts.md) |
| Move a session to another account | `ccx handoff`, `ccx -c`, `ccx --resume <id>` | [docs/handoff.md](docs/handoff.md) |
| Share the setup of the base account | `ccx manage sync`, `ccx manage sync --prefs` | [docs/sync.md](docs/sync.md) |
| Show and refresh the usage | `ccx status`, `ccx usage refresh`, `ccx manage service` | [docs/usage-data.md](docs/usage-data.md) |
| Add completions and aliases to your shell | `ccx manage shell-init` | [docs/shell.md](docs/shell.md) |
| Check the setup | `ccx manage doctor` | [docs/doctor.md](docs/doctor.md) |
| Change `config.toml` and `config.local.toml` | `ccx manage edit-config` ([Change an account](docs/accounts.md#change-an-account)) | [docs/configuration.md](docs/configuration.md) |

Architecture decisions: [docs/adr/](docs/adr/).

## Development

- [CONTRIBUTING.md](CONTRIBUTING.md): setup, checks, changes and releases.
- [AGENTS.md](AGENTS.md): the module map, the test method and the hard rules. `CLAUDE.md` imports it.

## Privacy rule

Do not put a real email address in this repository. Use `@example.com` addresses. The `check-emails` check fails on all other addresses, except addresses that contain `noreply`.

## License

MIT. Refer to [LICENSE](LICENSE).

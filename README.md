# ccx

ccx is a command-line tool. It starts Claude Code on the account that has the most usage available.

> **Status:** Work in progress. ccx does not launch Claude Code yet. Refer to [PLAN.md](PLAN.md) for the plan.

## How ccx works

- Each account has its own Claude Code configuration directory. The base account uses `~/.claude`.
- ccx sets `CLAUDE_CONFIG_DIR` for the account that it selects, and then starts `claude`.
- ccx does not read, keep or refresh Claude Code credentials. All logins go through `claude auth login`.

## Development

This project uses [mise](https://mise.jdx.dev) for all tools.

1. Install the tools: `mise install`.
2. Run all checks: `mise run ci`.

The checks are `fmt`, `lint`, `test` and `check-emails`. You can run each check with `mise run <name>`.

## Privacy rule

Do not put a real email address in this repository. Use `@example.com` addresses in code, tests and docs. The `check-emails` check fails on all other addresses, except addresses that contain `noreply`.

## License

MIT. Refer to [LICENSE](LICENSE).

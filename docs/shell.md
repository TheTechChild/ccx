# Add completions and aliases to your shell

`ccx manage shell-init` prints shell code: completions for `ccx` and one alias for each name in `aliases`. Add one line to your shell profile:

```sh
eval "$(ccx manage shell-init zsh)"     # ~/.zshrc, after compinit
eval "$(ccx manage shell-init bash)"    # ~/.bashrc
ccx manage shell-init fish | source     # ~/.config/fish/config.fish
```

Shell-init prints completions for `ccx` commands, `manage` subcommands and one `@name` token for each account. It prints an alias for each `aliases = [...]` name in `config.toml`: `aliases = ["claude2"]` in account `angel2` gives `alias claude2='ccx @angel2'`. Without `config.toml`, shell-init prints only completions.

## Alias rules

- Shell-init never makes an alias or a function for `claude`. Bare `claude` stays on the base account.
- Alias names use letters, digits and `_`, and not a digit first.
- Shell-init refuses `claude`, `ccx` and a name that two accounts use.
- A name that is not valid gives an error, no shell code and a non-zero exit code.

## Replace a hand-written alias

To replace `alias claude2='CLAUDE_CONFIG_DIR=~/.claude-angel2 claude'`:

1. Add `aliases = ["claude2"]` to the account in `config.toml`.
2. Add the shell-init line.
3. Remove the hand-written alias.
4. In a new shell, check that `alias claude2` shows `ccx @angel2` and `type claude` shows the `claude` program, not an alias.

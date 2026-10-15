# Add completions and aliases to your shell

`ccx manage shell-init` prints shell code. Add one line to your shell profile:

```sh
eval "$(ccx manage shell-init zsh)"     # ~/.zshrc. Put this line after compinit.
eval "$(ccx manage shell-init bash)"    # ~/.bashrc
ccx manage shell-init fish | source     # ~/.config/fish/config.fish
```

The output has two parts:

1. Completions for `ccx`: the commands, the `manage` subcommands and one `@name` token for each account.
2. One alias for each name in `aliases = [...]` in `config.toml`. For example, `aliases = ["claude2"]` in account `angel2` gives `alias claude2='ccx @angel2'`.

Shell-init never makes an alias or a function for `claude`. The bare `claude` command stays on the base account. An alias name must be a shell identifier: letters, digits and `_`, and not a digit first. Shell-init also refuses the names `claude` and `ccx`, and an alias name that two accounts use. When an alias name is not valid, shell-init prints an error and no shell code, and exits with a non-zero code. When `config.toml` does not exist, shell-init prints only the completions.

## Replace a hand-written alias

If your profile has a hand-written alias, for example `alias claude2='CLAUDE_CONFIG_DIR=~/.claude-angel2 claude'`, do these steps:

1. Add the alias name to the account in `config.toml`: `aliases = ["claude2"]`.
2. Add the shell-init line to your shell profile.
3. Remove the hand-written alias from your profile.
4. Start a new shell. Make sure that `alias claude2` shows `ccx @angel2`, and that `type claude` shows the `claude` program, not an alias.

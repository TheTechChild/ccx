# ccx shell-init (zsh). Generated; do not edit.
#compdef ccx

autoload -U is-at-least

_ccx() {
    typeset -A opt_args
    typeset -a _arguments_options
    local ret=1

    if is-at-least 5.2; then
        _arguments_options=(-s -S -C)
    else
        _arguments_options=(-s -C)
    fi

    local context curcontext="$curcontext" state line
    _arguments "${_arguments_options[@]}" : \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
'-V[Print version]' \
'--version[Print version]' \
":: :_ccx_commands" \
"*::: :->ccx" \
&& ret=0
    case $state in
    (ccx)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ccx-command-$line[1]:"
        case $line[1] in
            (manage)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_ccx__subcmd__manage_commands" \
"*::: :->manage" \
&& ret=0

    case $state in
    (manage)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ccx-manage-command-$line[1]:"
        case $line[1] in
            (adopt)
_arguments "${_arguments_options[@]}" : \
'-y[Confirm the changes without a prompt]' \
'--yes[Confirm the changes without a prompt]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(add)
_arguments "${_arguments_options[@]}" : \
'--dir=[The account dir. The default is \`~/.claude-NAME\`]:DIR:_default' \
'--email=[The email to give to \`claude auth login --email\`]:EMAIL:_default' \
'--key-command=[With \`--api-key\`\: get the key from the stdout of this shell command at each paid launch. ccx keeps no copy of the key]:COMMAND:_default' \
'--no-login[Stop after sync. Do not log in]' \
'--no-prefs[Do not copy the user MCP servers and the preferences of the base account after the login]' \
'-y[Confirm the plan without a prompt]' \
'--yes[Confirm the plan without a prompt]' \
'(--email --no-login --no-prefs)--api-key[Make an API-key account\: a paid fallback. Add reads the key with a hidden prompt, or one line of stdin, and keeps it in the OS secret store as \`ccx-NAME\`. There is no login]' \
'(--key-command)--key-file[With \`--api-key\`\: keep the key in \`~/.config/ccx/keys/NAME\` at mode 0600, not in the OS secret store]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':name -- The account name. It must match `^\[a-z0-9\]\[a-z0-9_-\]*$`:_default' \
&& ret=0
;;
(login)
_arguments "${_arguments_options[@]}" : \
'()--all[Log in to each account that \`claude auth status\` shows as not logged in. Then show a summary for each account]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
'::name -- The account to log in to:_default' \
&& ret=0
;;
(logout)
_arguments "${_arguments_options[@]}" : \
'-y[Log out without a prompt]' \
'--yes[Log out without a prompt]' \
'-h[Print help]' \
'--help[Print help]' \
':name -- The account to log out of:_default' \
&& ret=0
;;
(edit)
_arguments "${_arguments_options[@]}" : \
'--email=[The expected email of the login]:EMAIL:_default' \
'--weight=[The weight for automatic selection. It must be more than 0]:WEIGHT:_default' \
'(--disable)--enable[Use the account for automatic selection]' \
'--disable[Do not use the account for automatic selection]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':name -- The account to change:_default' \
&& ret=0
;;
(edit-config)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(rm)
_arguments "${_arguments_options[@]}" : \
'--purge[Also log out, delete the credentials file and move the dir into the trash]' \
'-y[Confirm the plan without a prompt]' \
'--yes[Confirm the plan without a prompt]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':name -- The account to remove:_default' \
&& ret=0
;;
(bootstrap)
_arguments "${_arguments_options[@]}" : \
'(--no-service)--service[Install the service without a prompt]' \
'--no-service[Do not install the service]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--json[Print stable JSON]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(shell-init)
_arguments "${_arguments_options[@]}" : \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':shell -- The shell:(zsh bash fish)' \
&& ret=0
;;
(sync)
_arguments "${_arguments_options[@]}" : \
'()--all[Sync all accounts other than the base account]' \
'--dry-run[Show the plan and change nothing]' \
'--force[Move CONFLICT items to the backup without a prompt]' \
'(--force)--prefs[Copy the user MCP servers and the allowlisted preferences of the base account. Do not link]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
'::name -- The account to sync:_default' \
&& ret=0
;;
(doctor)
_arguments "${_arguments_options[@]}" : \
'--json[Print the results as JSON]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
&& ret=0
;;
(service)
_arguments "${_arguments_options[@]}" : \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
":: :_ccx__subcmd__manage__subcmd__service_commands" \
"*::: :->service" \
&& ret=0

    case $state in
    (service)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ccx-manage-service-command-$line[1]:"
        case $line[1] in
            (install)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(uninstall)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--json[Print stable JSON]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
&& ret=0
;;
(usage)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_ccx__subcmd__usage_commands" \
"*::: :->usage" \
&& ret=0

    case $state in
    (usage)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ccx-usage-command-$line[1]:"
        case $line[1] in
            (refresh)
_arguments "${_arguments_options[@]}" : \
'()--all[Refresh all enabled accounts]' \
'--background[Run as the detached background refresh\: no output, no wait for the lock and the \`usage.refresh_min_gap_secs\` rule]' \
'-h[Print help]' \
'--help[Print help]' \
'::name -- The account to refresh:_default' \
&& ret=0
;;
        esac
    ;;
esac
;;
(handoff)
_arguments "${_arguments_options[@]}" : \
'--all[List the threads from all directories]' \
'--no-resume[Copy only. Do not start claude. Print the session id in the target account]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
'::id -- The session id, or a unique part of 8 or more characters at its start:_default' \
'::target -- The target account, for example `@angel2`:_default' \
&& ret=0
;;
(@angel2)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(@lab)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(@main)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
}

(( $+functions[_ccx_commands] )) ||
_ccx_commands() {
    local commands; commands=(
'manage:Manage accounts' \
'status:Show the kind, tier and usage of each account' \
'usage:Fetch usage and write the usage cache' \
'handoff:Copy a session to a different account and resume it there' \
'@angel2:Start claude as account angel2' \
'@lab:Start claude as account lab' \
'@main:Start claude as account main' \
    )
    _describe -t commands 'ccx commands' commands "$@"
}
(( $+functions[_ccx__subcmd__@angel2_commands] )) ||
_ccx__subcmd__@angel2_commands() {
    local commands; commands=()
    _describe -t commands 'ccx @angel2 commands' commands "$@"
}
(( $+functions[_ccx__subcmd__@lab_commands] )) ||
_ccx__subcmd__@lab_commands() {
    local commands; commands=()
    _describe -t commands 'ccx @lab commands' commands "$@"
}
(( $+functions[_ccx__subcmd__@main_commands] )) ||
_ccx__subcmd__@main_commands() {
    local commands; commands=()
    _describe -t commands 'ccx @main commands' commands "$@"
}
(( $+functions[_ccx__subcmd__handoff_commands] )) ||
_ccx__subcmd__handoff_commands() {
    local commands; commands=()
    _describe -t commands 'ccx handoff commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage_commands] )) ||
_ccx__subcmd__manage_commands() {
    local commands; commands=(
'adopt:Find the accounts on this machine and write them into the config' \
'add:Make a new account\: the dir, the sync links, the login and the config entry' \
'login:Log in to an account with \`claude auth login\`' \
'logout:Log out of an account with \`claude auth logout\`' \
'edit:Change the entry of an account and validate the config' \
'edit-config:Open \`config.toml\` in \`\$EDITOR\` and validate it after the editor exits. When the new text is not valid, the old file stays' \
'rm:Remove an account from the config. Keep its dir' \
'bootstrap:Set up ccx on a new machine in one command' \
'list:Show each configured account' \
'shell-init:Print shell code for the profile\: completions and account aliases' \
'sync:Link the shared setup of the base account into other accounts' \
'doctor:Check the setup and show the problems' \
'service:Install, remove or show the background usage refresh' \
    )
    _describe -t commands 'ccx manage commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__add_commands] )) ||
_ccx__subcmd__manage__subcmd__add_commands() {
    local commands; commands=()
    _describe -t commands 'ccx manage add commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__adopt_commands] )) ||
_ccx__subcmd__manage__subcmd__adopt_commands() {
    local commands; commands=()
    _describe -t commands 'ccx manage adopt commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__bootstrap_commands] )) ||
_ccx__subcmd__manage__subcmd__bootstrap_commands() {
    local commands; commands=()
    _describe -t commands 'ccx manage bootstrap commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__doctor_commands] )) ||
_ccx__subcmd__manage__subcmd__doctor_commands() {
    local commands; commands=()
    _describe -t commands 'ccx manage doctor commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__edit_commands] )) ||
_ccx__subcmd__manage__subcmd__edit_commands() {
    local commands; commands=()
    _describe -t commands 'ccx manage edit commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__edit-config_commands] )) ||
_ccx__subcmd__manage__subcmd__edit-config_commands() {
    local commands; commands=()
    _describe -t commands 'ccx manage edit-config commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__list_commands] )) ||
_ccx__subcmd__manage__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ccx manage list commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__login_commands] )) ||
_ccx__subcmd__manage__subcmd__login_commands() {
    local commands; commands=()
    _describe -t commands 'ccx manage login commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__logout_commands] )) ||
_ccx__subcmd__manage__subcmd__logout_commands() {
    local commands; commands=()
    _describe -t commands 'ccx manage logout commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__rm_commands] )) ||
_ccx__subcmd__manage__subcmd__rm_commands() {
    local commands; commands=()
    _describe -t commands 'ccx manage rm commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__service_commands] )) ||
_ccx__subcmd__manage__subcmd__service_commands() {
    local commands; commands=(
'install:Write the job for this system and load it' \
'uninstall:Stop the job and remove its files' \
'status:Show if the job is installed and loaded, the last refresh and the age of each usage cache' \
    )
    _describe -t commands 'ccx manage service commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__service__subcmd__install_commands] )) ||
_ccx__subcmd__manage__subcmd__service__subcmd__install_commands() {
    local commands; commands=()
    _describe -t commands 'ccx manage service install commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__service__subcmd__status_commands] )) ||
_ccx__subcmd__manage__subcmd__service__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'ccx manage service status commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__service__subcmd__uninstall_commands] )) ||
_ccx__subcmd__manage__subcmd__service__subcmd__uninstall_commands() {
    local commands; commands=()
    _describe -t commands 'ccx manage service uninstall commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__shell-init_commands] )) ||
_ccx__subcmd__manage__subcmd__shell-init_commands() {
    local commands; commands=()
    _describe -t commands 'ccx manage shell-init commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__sync_commands] )) ||
_ccx__subcmd__manage__subcmd__sync_commands() {
    local commands; commands=()
    _describe -t commands 'ccx manage sync commands' commands "$@"
}
(( $+functions[_ccx__subcmd__status_commands] )) ||
_ccx__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'ccx status commands' commands "$@"
}
(( $+functions[_ccx__subcmd__usage_commands] )) ||
_ccx__subcmd__usage_commands() {
    local commands; commands=(
'refresh:Run the \`/usage\` fetch and write \`~/.local/state/ccx/usage/<name>.json\`' \
    )
    _describe -t commands 'ccx usage commands' commands "$@"
}
(( $+functions[_ccx__subcmd__usage__subcmd__refresh_commands] )) ||
_ccx__subcmd__usage__subcmd__refresh_commands() {
    local commands; commands=()
    _describe -t commands 'ccx usage refresh commands' commands "$@"
}

if [ "$funcstack[1]" = "_ccx" ]; then
    _ccx "$@"
else
    (( $+functions[compdef] )) && compdef _ccx ccx
fi
alias claude2='ccx @angel2'
alias cl_lab='ccx @lab'
alias c3='ccx @lab'

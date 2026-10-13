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
'-h[Print help]' \
'--help[Print help]' \
'*::args:_default' \
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
'handoff:Move a session to a different account (not ready yet)' \
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
'list:Show each configured account' \
'shell-init:Print shell code for the profile\: completions and account aliases' \
'sync:Link the shared setup of the base account into other accounts' \
'doctor:Check the setup and show the problems' \
    )
    _describe -t commands 'ccx manage commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__adopt_commands] )) ||
_ccx__subcmd__manage__subcmd__adopt_commands() {
    local commands; commands=()
    _describe -t commands 'ccx manage adopt commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__doctor_commands] )) ||
_ccx__subcmd__manage__subcmd__doctor_commands() {
    local commands; commands=()
    _describe -t commands 'ccx manage doctor commands' commands "$@"
}
(( $+functions[_ccx__subcmd__manage__subcmd__list_commands] )) ||
_ccx__subcmd__manage__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ccx manage list commands' commands "$@"
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

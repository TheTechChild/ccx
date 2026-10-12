# ccx shell-init (fish). Generated; do not edit.
# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_ccx_global_optspecs
    string join \n h/help V/version
end

function __fish_ccx_needs_command
    # Figure out if the current invocation already has a command.
    set -l cmd (commandline -opc)
    set -e cmd[1]
    argparse -s (__fish_ccx_global_optspecs) -- $cmd 2>/dev/null
    or return
    if set -q argv[1]
        # Also print the command, so this can be used to figure out what it is.
        echo $argv[1]
        return 1
    end
    return 0
end

function __fish_ccx_using_subcommand
    set -l cmd (__fish_ccx_needs_command)
    test -z "$cmd"
    and return 1
    contains -- $cmd[1] $argv
end

complete -c ccx -n "__fish_ccx_needs_command" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c ccx -n "__fish_ccx_needs_command" -s V -l version -d 'Print version'
complete -c ccx -n "__fish_ccx_needs_command" -f -a "manage" -d 'Manage accounts'
complete -c ccx -n "__fish_ccx_needs_command" -f -a "status" -d 'Show the usage of each account (not ready yet)'
complete -c ccx -n "__fish_ccx_needs_command" -f -a "usage" -d 'Fetch and show usage (not ready yet)'
complete -c ccx -n "__fish_ccx_needs_command" -f -a "handoff" -d 'Move a session to a different account (not ready yet)'
complete -c ccx -n "__fish_ccx_needs_command" -f -a "@angel2" -d 'Start claude as account angel2'
complete -c ccx -n "__fish_ccx_needs_command" -f -a "@lab" -d 'Start claude as account lab'
complete -c ccx -n "__fish_ccx_needs_command" -f -a "@main" -d 'Start claude as account main'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt list shell-init" -s h -l help -d 'Print help'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt list shell-init" -f -a "adopt" -d 'Find the accounts on this machine and write them into the config'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt list shell-init" -f -a "list" -d 'Show each configured account'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt list shell-init" -f -a "shell-init" -d 'Print shell code for the profile: completions and account aliases'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from adopt" -s y -l yes -d 'Confirm the changes without a prompt'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from adopt" -s h -l help -d 'Print help'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from list" -l json -d 'Print stable JSON'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from shell-init" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c ccx -n "__fish_ccx_using_subcommand status" -s h -l help -d 'Print help'
complete -c ccx -n "__fish_ccx_using_subcommand usage" -s h -l help -d 'Print help'
complete -c ccx -n "__fish_ccx_using_subcommand handoff" -s h -l help -d 'Print help'
alias claude2 'ccx @angel2'
alias cl_lab 'ccx @lab'
alias c3 'ccx @lab'

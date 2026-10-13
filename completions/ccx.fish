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
complete -c ccx -n "__fish_ccx_needs_command" -f -a "status" -d 'Show the kind, tier and usage of each account'
complete -c ccx -n "__fish_ccx_needs_command" -f -a "usage" -d 'Fetch usage and write the usage cache'
complete -c ccx -n "__fish_ccx_needs_command" -f -a "handoff" -d 'Copy a session to a different account and resume it there'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt add login logout edit edit-config rm bootstrap list spend shell-init sync doctor service" -s h -l help -d 'Print help'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt add login logout edit edit-config rm bootstrap list spend shell-init sync doctor service" -f -a "adopt" -d 'Find the accounts on this machine and write them into the config'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt add login logout edit edit-config rm bootstrap list spend shell-init sync doctor service" -f -a "add" -d 'Make a new account: the dir, the sync links, the login and the config entry'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt add login logout edit edit-config rm bootstrap list spend shell-init sync doctor service" -f -a "login" -d 'Log in to an account with `claude auth login`'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt add login logout edit edit-config rm bootstrap list spend shell-init sync doctor service" -f -a "logout" -d 'Log out of an account with `claude auth logout`'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt add login logout edit edit-config rm bootstrap list spend shell-init sync doctor service" -f -a "edit" -d 'Change the entry of an account and validate the config'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt add login logout edit edit-config rm bootstrap list spend shell-init sync doctor service" -f -a "edit-config" -d 'Open `config.toml` in `$EDITOR` and validate it after the editor exits. When the new text is not valid, the old file stays'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt add login logout edit edit-config rm bootstrap list spend shell-init sync doctor service" -f -a "rm" -d 'Remove an account from the config. Keep its dir'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt add login logout edit edit-config rm bootstrap list spend shell-init sync doctor service" -f -a "bootstrap" -d 'Set up ccx on a new machine in one command'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt add login logout edit edit-config rm bootstrap list spend shell-init sync doctor service" -f -a "list" -d 'Show each configured account'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt add login logout edit edit-config rm bootstrap list spend shell-init sync doctor service" -f -a "spend" -d 'Show an estimate of the money that each API-key account used in a month'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt add login logout edit edit-config rm bootstrap list spend shell-init sync doctor service" -f -a "shell-init" -d 'Print shell code for the profile: completions and account aliases'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt add login logout edit edit-config rm bootstrap list spend shell-init sync doctor service" -f -a "sync" -d 'Link the shared setup of the base account into other accounts'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt add login logout edit edit-config rm bootstrap list spend shell-init sync doctor service" -f -a "doctor" -d 'Check the setup and show the problems'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and not __fish_seen_subcommand_from adopt add login logout edit edit-config rm bootstrap list spend shell-init sync doctor service" -f -a "service" -d 'Install, remove or show the background usage refresh'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from adopt" -l rename -d 'Adopt the found account OLD with the name NEW, for example `--rename angel2=work`. You can give it more than one time' -r
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from adopt" -l skip -d 'Do not adopt the found account NAME. You can give it more than one time' -r
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from adopt" -s y -l yes -d 'Confirm the changes without a prompt'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from adopt" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from add" -l dir -d 'The account dir. The default is `~/.claude-NAME`' -r
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from add" -l email -d 'The email to give to `claude auth login --email`' -r
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from add" -l key-command -d 'With `--api-key`: get the key from the stdout of this shell command at each paid launch. ccx keeps no copy of the key' -r
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from add" -l no-login -d 'Stop after sync. Do not log in'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from add" -l no-prefs -d 'Do not copy the user MCP servers and the preferences of the base account after the login'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from add" -s y -l yes -d 'Confirm the plan without a prompt'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from add" -l api-key -d 'Make an API-key account: a paid fallback. Add reads the key with a hidden prompt, or one line of stdin, and keeps it in the OS secret store as `ccx-NAME`. There is no login'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from add" -l key-file -d 'With `--api-key`: keep the key in `~/.config/ccx/keys/NAME` at mode 0600, not in the OS secret store'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from add" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from login" -l all -d 'Log in to each account that `claude auth status` shows as not logged in. Then show a summary for each account'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from login" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from logout" -s y -l yes -d 'Log out without a prompt'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from logout" -s h -l help -d 'Print help'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from edit" -l email -d 'The expected email of the login' -r
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from edit" -l weight -d 'The weight for automatic selection. It must be more than 0' -r
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from edit" -l enable -d 'Use the account for automatic selection'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from edit" -l disable -d 'Do not use the account for automatic selection'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from edit" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from edit-config" -s h -l help -d 'Print help'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from rm" -l purge -d 'Also log out, delete the credentials file and move the dir into the trash'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from rm" -s y -l yes -d 'Confirm the plan without a prompt'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from rm" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from bootstrap" -l service -d 'Install the service without a prompt'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from bootstrap" -l no-service -d 'Do not install the service'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from bootstrap" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from list" -l json -d 'Print stable JSON'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from spend" -l month -d 'The month in local time, as YYYY-MM. The default is the current month' -r
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from spend" -l json -d 'Print stable JSON'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from spend" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from shell-init" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from sync" -l all -d 'Sync all accounts other than the base account'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from sync" -l dry-run -d 'Show the plan and change nothing'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from sync" -l force -d 'Move CONFLICT items to the backup without a prompt'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from sync" -l prefs -d 'Copy the user MCP servers and the allowlisted preferences of the base account. Do not link'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from sync" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from doctor" -l json -d 'Print the results as JSON'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from doctor" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from service" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from service" -f -a "install" -d 'Write the job for this system and load it'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from service" -f -a "uninstall" -d 'Stop the job and remove its files'
complete -c ccx -n "__fish_ccx_using_subcommand manage; and __fish_seen_subcommand_from service" -f -a "status" -d 'Show if the job is installed and loaded, the last refresh, and the cache age and the next due time of each account'
complete -c ccx -n "__fish_ccx_using_subcommand status" -l json -d 'Print stable JSON'
complete -c ccx -n "__fish_ccx_using_subcommand status" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c ccx -n "__fish_ccx_using_subcommand usage; and not __fish_seen_subcommand_from refresh" -s h -l help -d 'Print help'
complete -c ccx -n "__fish_ccx_using_subcommand usage; and not __fish_seen_subcommand_from refresh" -f -a "refresh" -d 'Run the `/usage` fetch and write `~/.local/state/ccx/usage/<name>.json`'
complete -c ccx -n "__fish_ccx_using_subcommand usage; and __fish_seen_subcommand_from refresh" -l all -d 'Refresh all enabled accounts'
complete -c ccx -n "__fish_ccx_using_subcommand usage; and __fish_seen_subcommand_from refresh" -l background -d 'Run as the detached background refresh: no output, no wait for the lock and the `usage.refresh_min_gap_secs` rule'
complete -c ccx -n "__fish_ccx_using_subcommand usage; and __fish_seen_subcommand_from refresh" -l scheduled -d 'Run as the service job: a background refresh that fetches only the accounts that are due by the adaptive interval'
complete -c ccx -n "__fish_ccx_using_subcommand usage; and __fish_seen_subcommand_from refresh" -s h -l help -d 'Print help'
complete -c ccx -n "__fish_ccx_using_subcommand handoff" -l all -d 'List the threads from all directories'
complete -c ccx -n "__fish_ccx_using_subcommand handoff" -l no-resume -d 'Copy only. Do not start claude. Print the session id in the target account'
complete -c ccx -n "__fish_ccx_using_subcommand handoff" -s h -l help -d 'Print help (see more with \'--help\')'

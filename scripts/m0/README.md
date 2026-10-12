# M0 test scripts

These scripts help Clayton do M0 tests 2, 3, 5 and 6 of issue #2. Claude did tests 1 and 4. Refer to [ADR 0005](../../docs/adr/0005-m0-usage-hooks-off.md) and [ADR 0006](../../docs/adr/0006-m0-session-file-layout.md).

The scripts are throwaway tools. They are not part of ccx.

## Before you start

- Do the tests on macOS, in a terminal in the repository root.
- Make sure that `claude`, `jq` and `perl` are on your `PATH`. Install `jq` with mise if it is missing: `mise use -g jq`.
- The default accounts are `~/.claude` (base) and `~/.claude-angel2` (second account). Each script takes other directories as arguments.
- The scripts show no secrets and no file contents. Do not paste email addresses into the results.
- For each test, record the result, the `claude --version` output and the date. Give them to Claude. Claude adds them to the ADRs.

## Test 2: the `settings.json` symlink

This test finds if `/config` keeps the symlink `~/.claude-angel2/settings.json`.

1. Run `scripts/m0/test2-settings-symlink.sh`.
2. The script shows the state before the test. It must be `symlink -> ...`.
3. In a different terminal, do the steps that the script shows: start `claude` in angel2, change one setting with `/config`, and then `/exit`.
   - The script also asks you to change the setting back. If the checksum does not change, `/config` did not write the file. Then run the test again and do not change the setting back. After the test, change it back in the base account.
4. Go back to the first terminal and push Enter.
5. Record the `RESULT` line.

If the result is FAIL, the D4 fallback applies: `ccx manage sync` (#7) reports CONFLICT and relinks only with `--force`. To repair the link after the test, move the new file away and make the symlink again:

```sh
mv ~/.claude-angel2/settings.json ~/.claude-angel2/settings.json.m0-test2
ln -s ~/.claude/settings.json ~/.claude-angel2/settings.json
```

## Test 3: `/usage` fetch while a session is open

This test finds if a `/usage` fetch and an open session can both write `.claude.json` with no data loss.

1. Run `scripts/m0/test3-concurrent-usage.sh`.
2. In a different terminal, start `claude` in angel2 as the script shows. Send one short prompt. Keep the session open.
3. Go back to the first terminal and push Enter. The script runs 5 `/usage` fetches. Each line must show `usage data=yes` and `valid=yes`.
4. In the session, send one more prompt and then `/exit`.
5. Go back to the first terminal and push Enter.
6. Record the `RESULT` line and all lines that show `no` or a lost key.

There is no D4 fallback for test 3. If the result is FAIL, STOP and tell Claude.

## Test 5: resume in a different account after a copy

This test finds if `claude --resume <id>` works in angel2 after a copy of all session files from the base account.

1. Make a throwaway session in the base account:

   ```sh
   mkdir -p ~/ccx-m0-test5 && cd ~/ccx-m0-test5
   env -u CLAUDE_CONFIG_DIR claude
   ```

   Send this prompt: `Remember the word pineapple. Then make a file note.txt with the text hello.` Accept the file write. Then type `/exit`.
2. In the same directory, get the session id:

   ```sh
   ID=$(~/projects/ccx/scripts/m0/test5-resume-copy.sh latest)
   echo "$ID"
   ```

   Use the path of your clone of the ccx repository.
3. Wait 10 seconds. Then copy the session to angel2:

   ```sh
   ~/projects/ccx/scripts/m0/test5-resume-copy.sh copy "$ID"
   ```

   The script shows the files that it will copy. Type `y`. The script never overwrites a file, and it refuses if the session is open.
4. Do the steps that the script shows: `cd` to the session directory, then run `CLAUDE_CONFIG_DIR=~/.claude-angel2 claude --resume <id>`.
5. Make sure that the earlier messages are shown. Ask: `Which word did I ask you to remember?` Then type `/exit`.
6. Record: resume worked (yes or no), and the error text if there is one.
7. Remove the copy and the throwaway directory:

   ```sh
   ~/projects/ccx/scripts/m0/test5-resume-copy.sh undo "$ID"
   rm -rf ~/ccx-m0-test5
   ```

If the result is FAIL, the D4 fallback applies: #14 ships `list` and `copy` only, and the work stops before #15.

## Test 6: launchd job with a minimal `PATH`

This test finds if a launchd job can run `claude` and use the keychain login. The job has `PATH=/usr/bin:/bin:/usr/sbin:/sbin`.

1. Run `scripts/m0/test6-launchd.sh install`. The script installs the LaunchAgent `dev.ccx.m0-test6` and starts it one time.
2. If macOS shows a keychain dialog, record this. Then select **Allow** (not **Always Allow**).
3. Wait approximately 30 seconds.
4. Run `scripts/m0/test6-launchd.sh result`. If the script says that the job has not finished, wait and run it again.
5. Record the lines `A ...`, `B ...` and `C ...`, the `RESULT` line and the `Second account` line.
   - A: bare `claude` with the minimal `PATH`.
   - B: absolute path to `claude` with the minimal `PATH`.
   - C: absolute path to `claude` with angel2.
6. Run `scripts/m0/test6-launchd.sh uninstall`.

If A and B fail, the D4 fallback applies: #13 ships without launchd, and stale-while-revalidate (#11) stays the primary refresh. If only A fails, the service must use the absolute path to `claude`.

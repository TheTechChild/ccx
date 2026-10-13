# 0010. API-key accounts keep their keys through the OS commands

- **Status:** Accepted
- **Date:** 2026-10-12
- **Issue:** #20

## Context

Issue #20 adds API-key accounts as a paid fallback. ccx must keep the Anthropic API key of each such account in the OS secret store, under the service name `ccx-<name>`. The issue names the `keyring` crate for this.

ADR 0002 says that ccx never reads Claude Code credentials. An API key that ccx stores itself is a different secret: ccx writes it, and ccx is the only reader.

We examined the `keyring` crate:

- On Linux, the persistent Secret Service backend of the crate needs the `dbus` crate. That crate links `libdbus-1` and needs the development package at build time. The release builds `aarch64-unknown-linux-gnu` on a different machine. This makes the release build more difficult.
- On macOS, the keychain gives access to an item only to the program that made it. Each ccx update is a new unsigned binary, so macOS can ask for the keychain password after each update.
- The crate adds many dependencies to a tool that must start fast and have few dependencies.

## Decision

ccx uses the commands of the OS for the secret store:

- macOS: `security`. `security -i` reads the `add-generic-password` command from stdin, with the key as hex (`-X`). The key is never in the argument list of a process. `security find-generic-password -a ccx -s ccx-<name> -w` reads the key. `security delete-generic-password` deletes it.
- Linux: `secret-tool` from libsecret. `secret-tool store` reads the key from stdin. `lookup` and `clear` use the attributes `service ccx-<name>` and `account ccx`.

Each call first checks that the service name is `ccx-<name>` with a valid account name. A guard test (`tests/no_credentials.rs`) allows a keychain read only in `src/keystore.rs`, and checks that each call of `OsStore` does this check.

When the store is not available (for example on an SSH server with no D-Bus session), `ccx manage add --api-key` offers `key_command` or a key file at `~/.config/ccx/keys/<name>` with mode 0600, in a directory with mode 0700. ccx writes the file only after a confirm, and refuses to read a key file with a different mode.

## Consequences

- No new dependency, and no system library at build time.
- The `security` command owns the keychain item, so a ccx update does not cause a keychain prompt.
- On Linux, the user must install `secret-tool` (for example the `libsecret-tools` package) to use `key_store = "keyring"`. Without it, the store is not available, and ccx offers the two fallbacks.
- Tests use the fake `security` and `secret-tool` in `testdata/fakebin`, and an in-memory store in the unit tests. No test touches a real keychain.
- If Clayton prefers the `keyring` crate, only `OsStore` in `src/keystore.rs` must change.

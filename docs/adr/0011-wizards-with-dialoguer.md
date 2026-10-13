# 0011. The manage wizards use multi-step dialoguer prompts

- **Status:** Accepted
- **Date:** 2026-10-12
- **Issue:** #21
- **Decision:** D6

## Context

Issue #21 adds guided wizards to the `ccx manage` commands. A wizard asks for each value one step at a time, shows the current state, and shows a summary before it makes a change. The issue named three crates: `dialoguer`, `inquire` and `ratatui`.

ccx already uses `dialoguer` for its yes/no prompts and for the hidden API-key prompt. ccx must start fast and have few dependencies.

## Decision

The wizards use multi-step `dialoguer` prompts (decision D6). ccx does not add `inquire`, `ratatui` or a different TUI crate.

- A wizard is a sequence of `Input`, `Confirm` and `Select` prompts behind the `Prompt` trait in `src/wizard.rs`. The `Terminal` type implements it with `dialoguer`. The tests use a fake input source that gives the answers in order.
- A wizard only collects values. It gives the same value that the flags give: `add::Args`, `adopt::Choices`, `lifecycle::Edit`, or the name and the purge of `rm`. Then the command shows the summary, asks one last time, and changes the files. The flag path and the wizard path use the same plan and apply code.
- A wizard starts only when stdin, stdout and stderr are terminals and the command has no required argument: `manage add`, `manage edit` and `manage rm` with no NAME, and `manage adopt` with no `-y`, `--rename` or `--skip`. With no terminal, the command fails and names the missing flags. The flag interface does not change.
- The `rm` wizard asks the user to type the account name before a purge.
- `manage bootstrap` gets no wizard. Its only value is the service choice. Bootstrap already asks it on a terminal, and `--service` and `--no-service` give it in scripts.

## Reasons

- `dialoguer` is a dependency already. The wizards add no new crate and no new build step.
- The wizards are short sequences of questions. They need no full-screen layout, so a TUI crate gives no value for its size.
- The `Prompt` trait keeps the wizard logic testable without a terminal.

## Consequences

- Esc cancels a `Confirm` or a `Select` step. A `dialoguer` `Input` step does not read Esc. Ctrl-C stops the process at each step. In all cases the wizard has not changed a file, because the changes come only after the last confirm.
- `manage add` with a login runs `claude auth login` after the confirm. The summary with the email and the tier comes after the login.
- A full-screen TUI stays possible later behind the same `Prompt` trait.

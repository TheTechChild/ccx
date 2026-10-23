# 0014. ccx fills the accounts in a fixed order

- **Status:** Accepted
- **Date:** 2026-10-23
- **Supersedes:** the selection rule of the first build (#10) and decision 2 of [ADR 0013](0013-session-limit-awareness.md) (running sessions share the score)

## Context

The first build selected the account with the most usage available. The score of an account was its remaining capacity multiplied by a weight (Pro 1, Max 5x 5, Max 20x 20). The account key `weight` changed the weight. ccx stayed on the last account when its score was within `pick.stickiness` points of the best score.

This rule spreads the usage over all accounts. The maintainer wants the opposite: use one account fully, then the next account. The maintainer also does not want to keep an order in the config.

## Decision

ccx fills the accounts in a fixed order:

1. The base account (`~/.claude`). ccx finds it by its directory, not by its name.
2. The other subscription accounts, in name order.
3. The API-key accounts, always last.

Automatic selection uses the first subscription account in this order that is not limited. The limits (`pick.session_avoid`, `pick.weekly_avoid`, `pick.scoped_avoid`) do not change. When all subscription accounts are limited, ccx starts the account with the soonest reset, and can offer an API-key account (ADR 0010, #20). `@name` and `CCX_ACCOUNT` do not change.

## Consequences

- There is no score, no weight and no stickiness. Running sessions do not change the pick: ccx fills an account also when other sessions use it. The banner still shows the count, for example `1 running`. The named-launch warning and the limit hits of ADR 0013 do not change. `ccx manage edit --weight` is removed. `ccx manage add` shows the kind and the tier, but no weight.
- The config keys `accounts.<name>.weight` and `pick.stickiness` are retired. An old config with them still loads, and ccx ignores them. `ccx manage doctor` shows an INFO line for each retired key.
- When a window of an earlier account resets, the next launch goes back to that account.
- The automatic handoff on `ccx -c` and `ccx --resume <id>` uses the same order to select the target.
- ccx still writes the last pick to `~/.local/state/ccx/state.json`, but the picker does not read it.

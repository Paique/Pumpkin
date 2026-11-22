# GAP-0003: ADR-0006 names `/carrotgroups reload`, the code registers `/carrot groups reload`

- **Status:** Open
- **Found by:** ddd-update (doc/code reconciliation pass over Phases 1–5)
- **Date:** 2026-11-22

## Description

[ADR-0006](../decisions/ADR-0006-carrot-module-system.md), decision point 5, says group reload is `/carrotgroups reload`. The implemented code registers no `/carrotgroups` command at all — group reload is the `groups reload` subcommand of the root `/carrot` command (`carrot-core/src/command.rs`), i.e. `/carrot groups reload`. `docs/minecraft/commands.md` documents the command as the code registers it.

This is an implementation deviation from an accepted ADR that was never recorded: the ADR's decision text does not match the shipped behavior, and neither the Phase 1–2 commits nor any later commit records a superseding decision.

## Impact

Anyone reading ADR-0006 (the module-system contract) will type `/carrotgroups reload` and get an unknown-command error. The doc/code split also weakens the ADR as the authoritative record of the module system's surface.

## Proposed action

Pick one side (user decision, per the ddd-update rule that an unauthorized deviation must not be silently blessed either way):

- **Accept the code's shape** (recommended — root infrastructure commands keep one prefix, `/carrot`): supersede the wording by recording the decision in the ADR's next revision context (or a one-line correction note), and close this gap citing the record.
- **Match the ADR**: register `/carrotgroups` as an alias in `carrot-core/src/command.rs` and update `docs/minecraft/commands.md`.

## Resolution

*(pending user decision — 2026-11-22)*

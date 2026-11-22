# GAP-0001: No CI check verifies codegen output is in sync with `assets/`

- **Status:** Open
- **Found by:** ddd-setup
- **Date:** 2026-11-21

## Description

The docs record as an invariant that `crates/pumpkin-data/src/generated/` is always exactly the output of `pumpkin-codegen` running on the current `assets/` ([INV-0001](../invariants/INV-0001-generated-data-not-hand-edited.md)). However, no workflow in `.github/workflows/` (rust.yml, docker.yml, nightly.yml, nix.yml, release.yml, sync-wit.yml, typos.yml) regenerates the codegen output and compares it against the committed files — the absence was verified by grepping all workflows for `codegen`/`pumpkin-codegen`. A hand edit or a stale regeneration would pass CI today.

## Impact

INV-0001 is enforced by review only; drift between `assets/` and the generated tables can reach `master` undetected, and the next regeneration produces noisy diffs that are hard to attribute.

## Proposed action

Add a CI job (in `rust.yml` or a dedicated workflow) that runs `cargo run --locked -p pumpkin-codegen` on a clean checkout and fails when `git diff --exit-code -- crates/pumpkin-data/src/generated/` reports changes. Then flip INV-0001's status from `Documented-only` to `Enforced` and close this gap.

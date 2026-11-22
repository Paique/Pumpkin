# INV-0001: `crates/pumpkin-data/src/generated/` is never hand-edited

- **Status:** Documented-only
- **Scope:** `crates/pumpkin-data/src/generated/`, `tools/pumpkin-codegen`, anyone changing game data

## Statement

Every file under `crates/pumpkin-data/src/generated/` is byte-for-byte the output of `cargo run --locked -p pumpkin-codegen` running against the current `assets/` content. No human or agent edits files in that directory directly; changes land only by changing the inputs (`assets/` JSON or the codegen tool) and regenerating.

## Rationale

The generated tables encode version-pinned vanilla data ([ADR-0003](../decisions/ADR-0003-generated-game-data-pipeline.md)). A hand edit is untraceable to its data source, silently disagrees with the next regeneration, and turns every version bump into a manual merge of unknown quality. Regeneration is deterministic; hand edits are not.

## Enforcement

Currently **review-only**: the rule is stated in the root [AGENTS.md](../../../AGENTS.md) ("Game data and constants") and `tools/pumpkin-codegen/AGENTS.md`, but no CI step regenerates the output and fails on a diff. Cheapest enforcement point: a CI job that runs `cargo run --locked -p pumpkin-codegen` on a clean checkout and fails when `git diff --exit-code -- crates/pumpkin-data/src/generated/` is non-empty. Tracked as [GAP-0001](../gaps/GAP-0001-no-ci-check-for-codegen-drift.md).

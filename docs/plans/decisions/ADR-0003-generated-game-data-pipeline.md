# ADR-0003: Game data is generated from extracted vanilla data — never hardcoded

- **Status:** Accepted
- **Date:** 2026-11-21 (decision pre-dates this record; recorded by ddd-setup)
- **Deciders:** Pumpkin-MC maintainers (accepted before this record; CONTRIBUTING labels it "No Hardcoded Vanilla Values")

## Context

Block and item properties, entity dimensions, tags, recipes, loot tables, sounds and similar values change with every Minecraft update. Copies pasted into server code silently go stale on the next update. The pipeline in this repository: the external [Extractor](https://github.com/Pumpkin-MC/Extractor) (a Fabric mod that dumps data from the running game) produces the JSON in `assets/`; `tools/pumpkin-codegen` turns that JSON into `crates/pumpkin-data/src/generated/`. In addition, `crates/pumpkin-data/build.rs` extracts the vanilla datapack from a Minecraft server jar (downloaded from Mojang's piston-meta manifest or supplied via `PUMPKIN_MINECRAFT_SERVER_JAR`) into `assets/datapack/`. The rule is stated in [CONTRIBUTING.md](../../../CONTRIBUTING.md) ("No Hardcoded Vanilla Values") and AGENTS.md ("Game data and constants"); reviewers reject hardcoded copies.

## Decision

1. All game data values come from the generated tables in `pumpkin-data`. `crates/pumpkin-data/src/generated/` is codegen output and is never hand-edited ([INV-0001](../invariants/INV-0001-generated-data-not-hand-edited.md)).
2. A value present in `assets/` but not generated yet → extend `tools/pumpkin-codegen` and regenerate (`cargo run --locked -p pumpkin-codegen`).
3. A value not in `assets/` at all → belongs in the Extractor; prepare a change there under the same contribution rules.
4. **Exception:** values vanilla itself hardcodes as `static final` Java constants and that stay stable across versions remain named Rust constants, keeping the Java name. Engine-definable values (world height, dimension limits) are never guessed — datapacks can change them.

## Alternatives considered

- **Hardcoding values in source** — rejected: silently stale on every update; the exact failure mode this pipeline exists to prevent.
- **Reading values from a vanilla jar at runtime** — rejected: runtime dependency on Mojang distribution servers per server start, and datapack/datagen output already covers it at build time.

## Consequences

- Protocol/version bumps flow through Extractor → assets → codegen, not through scattered code edits; `CURRENT_MC_VERSION` and `LOWEST_SUPPORTED_MC_VERSION` live in the generated packet mappings (`pumpkin-data`).
- Contributors may need to touch two repositories (this one + Extractor) for a new data field.
- The generated layer is large and mechanical — diff review focuses on the codegen tool, not the output.

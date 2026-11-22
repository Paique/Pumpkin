# Generated game data

All version-dependent game data — block/item properties, entity dimensions, tags, recipes, loot tables, sounds, advancements, packet mappings — is **generated**, never written by hand: the external [Extractor](https://github.com/Pumpkin-MC/Extractor) (a Fabric mod dumping data from the running game) produces the JSON in `assets/`; `tools/pumpkin-codegen` turns it into `crates/pumpkin-data/src/generated/`; the datapack portion is extracted from a vanilla server jar at build time into `assets/datapack/`. Decision: [ADR-0003](../plans/decisions/ADR-0003-generated-game-data-pipeline.md).

## Vocabulary

- **assets/** — the extracted vanilla (and Bedrock) data dumps; the human-editable edge of the pipeline. Licenses/attribution: `assets/NOTICE.md`.
- **codegen** — `tools/pumpkin-codegen`; deterministic generator for `crates/pumpkin-data/src/generated/` (plain run) and for WIT-derived data/packet mappings (`-- wit`).
- **pumpkin-data** — the crate that *carries* the data; contains logic around it but no gameplay behaviour.
- **Vanilla datapack extraction** — `crates/pumpkin-data/build.rs` downloading (or taking locally via `PUMPKIN_MINECRAFT_SERVER_JAR`) the vanilla server jar from Mojang's piston-meta manifest at **build time**.

## Rules

- `crates/pumpkin-data/src/generated/` is never hand-edited ([INV-0001](../plans/invariants/INV-0001-generated-data-not-hand-edited.md)); regenerate instead (`cargo run --locked -p pumpkin-codegen`).
- A value in `assets/` but not generated yet → extend codegen. A value not in `assets/` at all → belongs in the Extractor repo first.
- The exception is vanilla `static final` Java constants: they stay as named Rust constants using the Java name, never bare numbers.
- Engine-definable values (world height, dimension limits) are read from where the game defines them — never guessed.

## Boundaries

This concept covers **data**, not behaviour: registries and tables, not the code acting on them. `pumpkin-data` containing no logic is a responsibility rule ([responsibilities.md](../architecture/responsibilities.md)). The WIT-derived generated data (packet mappings, bindings) follows the same "regenerate, don't edit" rule but belongs to the plugin platform concept.

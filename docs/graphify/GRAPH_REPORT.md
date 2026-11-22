# Graph Report - Pumpkin  (2026-11-21)

## Corpus Check
- Corpus is ~9,696 words - fits in a single context window. You may not need a graph.

## Summary
- 77 nodes · 221 edges · 8 communities (7 shown, 1 thin omitted)
- Extraction: 9% EXTRACTED · 91% INFERRED · 0% AMBIGUOUS · INFERRED: 202 edges (avg confidence: 0.9)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- development
- architecture
- concepts
- concepts
- concepts
- architecture
- concepts
- concepts

## God Nodes (most connected - your core abstractions)
1. `Plugin platform (Wasm)` - 9 edges
2. `pumpkin-data` - 8 edges
3. `ADR-0001: Runtime separation — sync tick, async Tokio, CPU work on Rayon` - 7 edges
4. `ADR-0004: Entity islands tick in parallel, bounded by reach reports` - 7 edges
5. `Vanilla datapack extraction` - 7 edges
6. `Wasm plugin host` - 6 edges
7. `WIT` - 6 edges
8. `Reach` - 6 edges
9. `ADR-0002: Player movement is client-authoritative` - 6 edges
10. `INV-0005: GPL server code never moves into permissively licensed plugin crates` - 6 edges

## Surprising Connections (you probably didn't know these)
- `INV-0004: CI compiles warning-free` --conceptually_related_to--> `INV-0003: Synchronous locks are never held across `.await``  [INFERRED]
  docs/plans/invariants/INV-0004-zero-warnings-ci.md → docs/plans/invariants/INV-0003-no-sync-locks-held-across-await.md
- `Plugin flow` --conceptually_related_to--> `Plugin platform (Wasm)`  [INFERRED]
  docs/architecture/flows.md → docs/concepts/plugin-platform.md
- `INV-0002: Server code never unwraps, expects, panics, todos or prints to std streams` --conceptually_related_to--> `INV-0004: CI compiles warning-free`  [INFERRED]
  docs/plans/invariants/INV-0002-no-panic-unwrap-stdout-in-server-code.md → docs/plans/invariants/INV-0004-zero-warnings-ci.md
- `Threading model` --conceptually_related_to--> `ADR-0001: Runtime separation — sync tick, async Tokio, CPU work on Rayon`  [INFERRED]
  docs/architecture/flows.md → docs/plans/decisions/ADR-0001-runtime-separation-sync-tick-async-io-rayon-cpu.md
- `Entity islands` --conceptually_related_to--> `ADR-0004: Entity islands tick in parallel, bounded by reach reports`  [INFERRED]
  docs/concepts/entity-islands-and-reach.md → docs/plans/decisions/ADR-0004-entity-islands-with-reach-reporting.md

## Hyperedges (group relationships)
- **Generated game data: ADR-0003 decision grounds INV-0001, whose review-only enforcement is tracked by GAP-0001** — docs_plans_decisions_adr_0003, docs_plans_invariants_inv_0001, docs_plans_gaps_gap_0001, docs_concepts_generated_game_data [EXTRACTED]
- **INV-0001 generated data not hand-edited is enforced across build, responsibilities and generated-game-data docs** — docs_plans_invariants_inv_0001, docs_development_build, docs_architecture_responsibilities, docs_concepts_generated_game_data [EXTRACTED]
- **INV-0005 plugin crate license boundary spans dependencies, responsibilities and plugin platform docs** — docs_plans_invariants_inv_0005, docs_architecture_dependencies, docs_architecture_responsibilities, docs_concepts_plugin_platform_license_boundary [EXTRACTED]
- **WIT changes require the host crates, the SDK and the runtime to change together** — docs_concepts_plugin_platform_wit, docs_concepts_plugin_platform_wasm_plugin_host, docs_concepts_plugin_platform_sdk, docs_concepts_plugin_platform_runtime [INFERRED]

## Communities (8 total, 1 thin omitted)

### Community 0 - "development"
Cohesion: 0.25
Nodes (9): Boot test, Headless bots, Clippy, typos, Criterion benches, pumpkin-fuzzer, Test harness, INV-0002: Server code never unwraps, expects, panics, todos or prints to std streams (+1 more)

### Community 1 - "architecture"
Cohesion: 0.29
Nodes (10): Allowed internal direction, Cargo workspace, GameTest framework, Generated game data, Runtime, SDK, Wasm plugin host, WIT (+2 more)

### Community 2 - "concepts"
Cohesion: 0.31
Nodes (12): Threading model, Audited reach, Conservative fallback, Entity islands, GoalReach, Reach, Runtime separation, Audited reach (+4 more)

### Community 3 - "concepts"
Cohesion: 0.31
Nodes (9): Chunk pipeline, No hardcoded vanilla values, Client-authoritative movement, Documented deviation, Port, Vanilla parity, Client-authoritative movement, ADR-0002: Player movement is client-authoritative (+1 more)

### Community 4 - "concepts"
Cohesion: 0.39
Nodes (8): Plugin flow, License boundary, Plugin data folder, Plugin platform (Wasm), License boundary, WIT contract, ADR-0005: Plugins run as Wasm against a public, mirrored WIT contract, INV-0005: GPL server code never moves into permissively licensed plugin crates

### Community 5 - "architecture"
Cohesion: 0.43
Nodes (6): Auxiliary listeners, Boot flow, Console commands, Java client connection, Crash reports, --convert-world

### Community 6 - "concepts"
Cohesion: 0.52
Nodes (6): assets/, Extractor, Vanilla datapack extraction, ADR-0003: Game data is generated from extracted vanilla data — never hardcoded, GAP-0001: No CI check verifies codegen output is in sync with `assets/`, INV-0001: `crates/pumpkin-data/src/generated/` is never hand-edited

## Knowledge Gaps
- **1 isolated node(s):** `Port`
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 1 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `ADR-0001: Runtime separation — sync tick, async Tokio, CPU work on Rayon` connect `concepts` to `concepts`, `architecture`?**
  _High betweenness centrality (0.078) - this node is a cross-community bridge._
- **Are the 9 inferred relationships involving `Plugin platform (Wasm)` (e.g. with `development/setup.md` and `architecture/dependencies.md`) actually correct?**
  _`Plugin platform (Wasm)` has 9 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Port` to the rest of the system?**
  _1 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Why does `Plugin platform (Wasm)` connect `concepts` to `development`, `architecture`, `concepts`, `architecture`?**
  _High betweenness centrality (0.077) - this node is a cross-community bridge._
- **Are the 8 inferred relationships involving `pumpkin-data` (e.g. with `architecture/dependencies.md` and `development/test.md`) actually correct?**
  _`pumpkin-data` has 8 INFERRED edges - model-reasoned connections that need verification._
- **Why does `ADR-0002: Player movement is client-authoritative` connect `concepts` to `architecture`?**
  _High betweenness centrality (0.037) - this node is a cross-community bridge._
- **Are the 5 inferred relationships involving `ADR-0001: Runtime separation — sync tick, async Tokio, CPU work on Rayon` (e.g. with `architecture/flows.md` and `architecture/responsibilities.md`) actually correct?**
  _`ADR-0001: Runtime separation — sync tick, async Tokio, CPU work on Rayon` has 5 INFERRED edges - model-reasoned connections that need verification._
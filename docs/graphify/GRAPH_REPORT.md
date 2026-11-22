# Graph Report - docs  (2026-11-22)

## Corpus Check
- 30 files · ~0 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 181 nodes · 313 edges · 18 communities (10 shown, 8 thin omitted)
- Extraction: 94% EXTRACTED · 6% INFERRED · 0% AMBIGUOUS · INFERRED: 18 edges (avg confidence: 0.82)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- architecture
- development
- concepts
- architecture
- architecture
- plans
- concepts
- architecture
- plans
- plans
- plans
- plans
- architecture
- architecture
- architecture
- architecture
- architecture
- plans

## God Nodes (most connected - your core abstractions)
1. `Generated game data pipeline` - 15 edges
2. `Wasm plugin platform` - 15 edges
3. `crates/carrot-core` - 13 edges
4. `Entity islands` - 12 edges
5. `crates/carrot (aggregator)` - 11 edges
6. `Carrot module system` - 11 edges
7. `ADR-0006 Carrot module system` - 10 edges
8. `crates/pumpkin-data` - 10 edges
9. `crates/pumpkin-core` - 9 edges
10. `crates/carrot-terrain` - 9 edges

## Surprising Connections (you probably didn't know these)
- `Tokio + Rayon + rustls runtime model` --semantically_similar_to--> `Threading model (sync tick, Tokio IO, Rayon CPU)`  [INFERRED] [semantically similar]
  docs/architecture/dependencies.md → docs/architecture/flows.md
- `Gameplay runs synchronously inside the tick` --semantically_similar_to--> `Threading model (sync tick, Tokio IO, Rayon CPU)`  [INFERRED] [semantically similar]
  docs/architecture/responsibilities.md → docs/architecture/flows.md
- `Server CLI flags (--init-settings, --convert-world)` --references--> `crates/pumpkin-world`  [INFERRED]
  docs/development/run.md → docs/architecture/modules.md
- `crates/carrot (aggregator)` --references--> `builder_mode module (creative-on-join)`  [EXTRACTED]
  docs/architecture/modules.md → docs/minecraft/commands.md
- `plugin-v0_2 cargo feature` --references--> `Wasm plugin platform`  [INFERRED]
  docs/development/build.md → docs/concepts/plugin-platform.md

## Import Cycles
- None detected.

## Hyperedges (group relationships)
- **Carrot module layer (fork feature crates + lifecycle)** — docs_architecture_modules_carrot_core, docs_architecture_modules_carrot_aggregator, docs_architecture_modules_carrot_essentials, docs_architecture_modules_carrot_chat, docs_architecture_modules_carrot_multiverse, docs_architecture_modules_carrot_worldedit, docs_architecture_modules_carrot_terrain, docs_architecture_flows_carrot_module_lifecycle, plans_decisions_adr0006_carrot_module_system [EXTRACTED 1.00]
- **Wasm plugin platform stack (WIT, SDK, hosts, runtime, decision)** — docs_concepts_plugin_platform_wasm_plugin_platform, docs_concepts_plugin_platform_wit, docs_concepts_plugin_platform_sdk, docs_concepts_plugin_platform_host, docs_concepts_plugin_platform_plugin_runtime, docs_architecture_modules_pumpkin_plugin_wit, docs_architecture_modules_pumpkin_plugin_api, docs_architecture_modules_pumpkin_plugin_runtime, docs_architecture_modules_pumpkin_wasm_host, docs_architecture_modules_pumpkin_wasm_host_common, docs_architecture_modules_pumpkin_wasm_host_v0_1, docs_architecture_modules_pumpkin_wasm_host_v0_2, docs_architecture_dependencies_wasmtime, plans_decisions_adr0005_wasm_plugin_platform_public_wit [EXTRACTED 1.00]
- **Generated game data pipeline (Extractor -> assets -> codegen -> pumpkin-data)** — docs_concepts_generated_game_data_pipeline, docs_concepts_generated_game_data_assets_directory, docs_concepts_generated_game_data_datapack_extraction, docs_architecture_modules_pumpkin_codegen, docs_architecture_modules_pumpkin_data, docs_architecture_dependencies_vanilla_extractor, docs_architecture_dependencies_mojang_piston_meta, plans_decisions_adr0003_generated_game_data_pipeline, plans_invariants_inv0001_generated_data_not_hand_edited [EXTRACTED 1.00]
- **Three execution domains (sync tick, Tokio async, Rayon CPU)** — docs_plans_decisions_adr_0001_runtime_separation_sync_tick_async_io_rayon_cpu_runtime_separation, docs_plans_decisions_adr_0001_runtime_separation_sync_tick_async_io_rayon_cpu_sync_tick_game_logic, docs_plans_decisions_adr_0001_runtime_separation_sync_tick_async_io_rayon_cpu_async_io_on_tokio, docs_plans_decisions_adr_0001_runtime_separation_sync_tick_async_io_rayon_cpu_rayon_cpu_work [EXTRACTED 1.00]
- **Wasm plugin platform layer stack (WIT, hosts, SDK, CI mirror)** — docs_plans_decisions_adr_0005_wasm_plugin_platform_public_wit_wasm_plugin_platform, docs_plans_decisions_adr_0005_wasm_plugin_platform_public_wit_public_wit_contract, docs_plans_decisions_adr_0005_wasm_plugin_platform_public_wit_pumpkin_plugin_wit, docs_plans_decisions_adr_0005_wasm_plugin_platform_public_wit_pumpkin_wasm_host, docs_plans_decisions_adr_0005_wasm_plugin_platform_public_wit_pumpkin_plugin_api, docs_plans_decisions_adr_0005_wasm_plugin_platform_public_wit_sync_wit_yml [EXTRACTED 1.00]
- **Generated game data pipeline (Extractor, codegen, generated tables)** — docs_plans_decisions_adr_0003_generated_game_data_pipeline_generated_game_data_pipeline, docs_plans_decisions_adr_0003_generated_game_data_pipeline_extractor, docs_plans_decisions_adr_0003_generated_game_data_pipeline_pumpkin_codegen, docs_plans_decisions_adr_0003_generated_game_data_pipeline_pumpkin_data_generated_tables [EXTRACTED 1.00]

## Communities (18 total, 8 thin omitted)

### Community 0 - "architecture"
Cohesion: 0.12
Nodes (24): Dependency layering rule (leaves -> domains -> core -> binary), pumpkin-plugin-wit mirror (Pumpkin-MC/pumpkin-plugin-wit), Wasmtime Wasm runtime, Plugin flow, pumpkin-core/src/plugin/permissions.rs, WasmPluginLoader, crates/pumpkin-plugin-api, crates/pumpkin-plugin-runtime (+16 more)

### Community 1 - "development"
Cohesion: 0.12
Nodes (18): tools/pumpkin-fuzzer, crates/pumpkin-gametest, tools/pumpkin-tester, CI platform matrix, console-subscriber cargo feature, plugin-v0_2 cargo feature, Release profile (fat LTO), Boot test baseline loop (+10 more)

### Community 2 - "concepts"
Cohesion: 0.17
Nodes (17): Mojang piston-meta / piston-data, Vanilla Extractor repository, VanillaData::load, tools/pumpkin-codegen, crates/pumpkin-data, No hardcoded vanilla values, assets/ (extracted vanilla data), Vanilla datapack extraction (pumpkin-data build.rs) (+9 more)

### Community 3 - "architecture"
Cohesion: 0.18
Nodes (20): Tokio + Rayon + rustls runtime model, crates/carrot (aggregator), crates/carrot-chat, crates/carrot-core, crates/carrot-essentials, crates/carrot-multiverse, crates/carrot-terrain, crates/carrot-worldedit (+12 more)

### Community 4 - "architecture"
Cohesion: 0.14
Nodes (17): Mojang session/registration services, Boot flow, Console commands flow, Java client connection flow (status -> login -> play), PumpkinServer, crates/pumpkin-auth, crates/pumpkin (binary), crates/pumpkin-command (+9 more)

### Community 5 - "plans"
Cohesion: 0.16
Nodes (18): PLUGIN_API_VERSION, Public WIT contract, pumpkin-plugin-api, pumpkin-plugin-wit, pumpkin-wasm-host, sync-wit.yml, Wasm plugin platform, builder_mode module (+10 more)

### Community 6 - "concepts"
Cohesion: 0.22
Nodes (12): Chunk pipeline, Level::fetch_chunks channel pattern, Threading model (sync tick, Tokio IO, Rayon CPU), Gameplay runs synchronously inside the tick, Audited reach (Mob::has_audited_tick), Conservative fallback reach, Entity islands, GoalReach (+4 more)

### Community 7 - "architecture"
Cohesion: 0.27
Nodes (11): Carrot layer one-way downward direction, Carrot groups system (groups.toml), carrot::init, Carrot module lifecycle, Carrot module system (runtime-toggleable feature layer), Carrot must not modify the upstream tree, builder_mode module (creative-on-join), /carrot root command (+3 more)

### Community 8 - "plans"
Cohesion: 0.21
Nodes (12): Async IO on Tokio, Level::fetch_chunks, Rayon CPU work, Runtime separation, Sync tick game logic, Entity islands, entity_islands.rs, Goal::tick_reach (+4 more)

### Community 9 - "plans"
Cohesion: 0.73
Nodes (6): Extractor, Generated game data pipeline, pumpkin-codegen, pumpkin-data generated tables, No CI check for codegen drift, Generated data is never hand-edited

## Knowledge Gaps
- **30 isolated node(s):** `crates/pumpkin-protocol`, `crates/pumpkin-auth`, `crates/pumpkin-macros`, `crates/pumpkin-api-macros`, `crates/pumpkin-inventory` (+25 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **8 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Entity islands` connect `concepts` to `plans`, `architecture`?**
  _High betweenness centrality (0.308) - this node is a cross-community bridge._
- **Why does `Entity islands` connect `plans` to `concepts`?**
  _High betweenness centrality (0.251) - this node is a cross-community bridge._
- **Are the 3 inferred relationships involving `Generated game data pipeline` (e.g. with `VanillaData::load` and `Vanilla parity (behavioural definition of correctness)`) actually correct?**
  _`Generated game data pipeline` has 3 INFERRED edges - model-reasoned connections that need verification._
- **What connects `crates/pumpkin-protocol`, `crates/pumpkin-auth`, `crates/pumpkin-macros` to the rest of the system?**
  _30 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `architecture` be split into smaller, more focused modules?**
  _Cohesion score 0.12307692307692308 - nodes in this community are weakly interconnected._
- **Should `development` be split into smaller, more focused modules?**
  _Cohesion score 0.11594202898550725 - nodes in this community are weakly interconnected._
- **Should `architecture` be split into smaller, more focused modules?**
  _Cohesion score 0.1437908496732026 - nodes in this community are weakly interconnected._
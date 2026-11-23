# Modules — workspace crate boundaries

> **Status:** Implemented — verified against the workspace `Cargo.toml` members list, each crate's `src/` layout and manifests (2026-11-22; carrot crates verified 2026-11-22 against commits `27da4e5af`–`a4fcb98b7`).

Pumpkin is a single Cargo workspace. The binary crate `pumpkin` wires together the library crates in `crates/`; three helper crates live in `tools/`. One directory in `crates/` is **not** a workspace member: `crates/pumpkin-plugin-wit` holds only WIT package files and licenses (no `Cargo.toml`) and is subtree-mirrored to its own repository on every push to `master` (`.github/workflows/sync-wit.yml`).

## Server

| Crate | Role |
| --- | --- |
| `crates/pumpkin` | The binary. `main.rs` loads config, sets up the logger, panic/crash reporting and signal handlers, wires plugin loaders, starts `PumpkinServer`. Also hosts two CLI-only modes: `--init-settings` / `--initSettings` and `--convert-world`. |
| `crates/pumpkin-core` | The server itself: tick loop and orchestration (`server/`, `PumpkinServer`), entities and AI (`entity/`), block behaviour (`block/`), items (`item/`), commands wiring (`command/`), networking (`net/`), plugin loading and permissions (`plugin/`), world coordination (`world/`, including `entity_islands.rs`), crash reports (`crash.rs`), telemetry, update checker, logger setup. |
| `crates/pumpkin-config` | All configuration schemas and TOML loading/merging for `pumpkin.toml` (basic, advanced, telemetry; networking, logging, plugins, world, …). Writes defaults back to disk when keys are missing. |
| `crates/pumpkin-protocol` | Wire protocols: Java Edition (`java/`), Bedrock Edition in progress (`bedrock/`), the codec layer (`codec/`, `ser/`, `serial/`), server query (`query.rs`). |
| `crates/pumpkin-auth` | Session/registration HTTP clients (`client.rs`, reqwest-based) and JWT handling (`jwt/`) used for online-mode authentication. |
| `crates/pumpkin-world` | Chunks and everything around them: chunk IO formats — vanilla Anvil, Linear, Pump (`chunk/`, `chunk/io/`), chunk system/scheduling (`chunk_system/`), world generation (`generation/`), lighting (`lighting/`), POI, ticking, level/world info, dimension and biome definitions, block entities' storage side. Also the world-format converter used by `--convert-world`. |

## Game data

| Crate | Role |
| --- | --- |
| `crates/pumpkin-data` | Game constants and registries: blocks, items, entities, recipes, advancements, packet mappings, Bedrock mappings. `src/generated/` is produced by `tools/pumpkin-codegen` from `assets/` — never hand-edit it. Its `build.rs` extracts the vanilla datapack from a Minecraft server jar (downloaded from Mojang's piston-meta manifest, or from a local jar via `PUMPKIN_MINECRAFT_SERVER_JAR`) into `assets/datapack/`. Criterion benches included. |
| `crates/pumpkin-macros` | Proc macros used across the server (e.g. `#[pumpkin_block(...)]` producing `BlockMetadata`). |
| `crates/pumpkin-api-macros` | Proc macros for the plugin-facing API surface. |

## Gameplay domain crates

| Crate | Role |
| --- | --- |
| `crates/pumpkin-inventory` | Containers, screen handlers, crafting, anvil, brewing, enchanting, merchant, player inventory, click/drag handling. |
| `crates/pumpkin-command` | The command tree engine, Brigadier-style: dispatcher, argument types, parsers, suggestions, command context, SNBT parsing. Vanilla `/`-commands themselves are registered in `pumpkin-core` (`command/commands/`). |
| `crates/pumpkin-gametest` | Mojang GameTest framework implementation used by the test harness. |

## Utility crates (leaves)

| Crate | Role |
| --- | --- |
| `crates/pumpkin-util` | Shared utilities: math, noise, random, identifiers/resource locations, text components and translations, registries, permissions, gamemode/difficulty enums, world seed, loot tables, version. Criterion benches included. |
| `crates/pumpkin-nbt` | NBT reading/writing: compound, serializer/deserializer, CESU-8, compression. Criterion benches included. |
| `crates/pumpkin-codecs` | Codec abstraction (the vanilla-data-serializer style): dynamic ops, JSON ops, struct builder, list/map builders, lifecycles. |
| `crates/pumpkin-scheduler` | Task scheduling backends, domains, global scheduler. |

## Plugin platform

| Crate | Role |
| --- | --- |
| `crates/pumpkin-plugin-wit` | WIT package sources for API versions v0.1 and v0.2. Not a Rust crate. Mirrored to `Pumpkin-MC/pumpkin-plugin-wit` on push to `master`; external SDK bindings are generated from it. Dual-licensed MIT OR Apache-2.0 (plus a Microsoft MIT variant file). |
| `crates/pumpkin-plugin-api` | The plugin SDK (permissive licensed; the only place plugin authors take a dependency). No internal Pumpkin dependencies. |
| `crates/pumpkin-plugin-runtime` | Plugin process/task runtime: lifecycle, executor, spawn policy, chains. |
| `crates/pumpkin-plugin-utils` | Plugin-side utilities: licensing, lease caching, update checks, crashlytics. |
| `crates/pumpkin-wasm-host` | Facade for the WebAssembly plugin host (`WasmPluginLoader`). |
| `crates/pumpkin-wasm-host-common` | Shared host plumbing. |
| `crates/pumpkin-wasm-host-v0_1` / `-v0_2` | Host implementations that generate their own bindings for WIT API v0.1 / v0.2. v0.2 is gated behind the `plugin-v0_2` feature of `pumpkin` (off by default). |

## Carrot modules (this fork)

Carrot is this fork's runtime-toggleable feature layer — the native replacement for the reference build server's Paper plugins ([ADR-0006](../plans/decisions/ADR-0006-carrot-module-system.md)). It lives in `crates/carrot*`, kept out of the upstream tree so rebases stay cheap; the only upstream-tree change is the `carrot::init` call in `crates/pumpkin/src/main.rs` plus workspace/member entries. Command surface: [docs/minecraft/commands.md](../minecraft/commands.md).

| Crate | Role |
| --- | --- |
| `crates/carrot-core` | The module system: `CarrotModule` trait (id, `depends()`, `default_enabled()`, sync `on_enable`/`on_disable`), `ModuleContext` (registers commands/events/permissions as a synthetic plugin context `carrot-<id>`), and the `Carrot` manager that loads `carrot.toml`/`groups.toml` from `<exec_dir>/carrot/`, enables modules in dependency order, and toggles them at runtime. Also owns the root `/carrot` command and the `carrot:` permission namespace with LuckPerms-style groups. |
| `crates/carrot` | The aggregator: registers `builder_mode` (creative-on-join for `carrot:buildermode` holders) and calls each feature crate's registration into the manager; `carrot::init(server, exec_dir)` is the single entry point. |
| `crates/carrot-essentials` | Player QoL: homes, warps, tpa, `/back`, `/spawn`, fly, speed, gamemode, night vision (`/n`, `/nv`). Persisted as write-through TOML under `carrot/modules/essentials/`. |
| `crates/carrot-chat` | Channel chat: local/global channels with config-driven command names (default `/l`, `/g`), pt-BR message filters, cooldowns and a local radius. Cancels vanilla broadcasts while enabled. |
| `crates/carrot-multiverse` | Multiworld: declare worlds in `worlds.toml`, load/unload at runtime, `/goto` teleports with on-demand loading. |
| `crates/carrot-worldedit` | The world-editing engine: per-player sessions (selection, clipboard, gmask) in memory, paginated on-disk undo/redo (`.cwh` v2: gzipped 500k-entry NBT pages), Sponge `.schem` I/O, patterns and masks, and the classic worldedit command set — all edits run through a per-player-lane, round-robin incremental queue that slices huge edits into bounded pieces (diff streamed to history pages, apply page by page under per-slice chunk leases). |
| `crates/carrot-terrain` | Terraforming on top of the worldedit engine (depends on `carrot-worldedit`'s `WeState`): png-heightmap brushes (goBrush), VoxelSniper-style tools, Arceon brushes/TypeReplace/Revolve/Loft, ezEdits texturing (`//eztexture`), terrain masks registered into the worldedit mask parser. 18 default brush heightmaps are compiled in via `include_bytes!` and seeded to `carrot/gobrush/brushes/` (runtime brushes are read from disk, so users can add their own). |

## Tools

| Path | Role |
| --- | --- |
| `tools/pumpkin-codegen` | Generates `pumpkin-data/src/generated/` from the JSON in `assets/`; `-- wit` regenerates WIT-derived data and packet mappings. Has its own `AGENTS.md`. |
| `tools/pumpkin-tester` | Test harness: headless protocol bots (`bot.rs`, `bot_tests/`) plus Mojang GameTests (`gametests.rs`) run with `cargo run -p pumpkin-tester`. |
| `tools/pumpkin-fuzzer` | Standalone network packet fuzzer/stress tool (clap CLI) targeting a running server. Deliberately relaxes the workspace clippy denies. |

## Assets

`assets/` holds extracted vanilla/Bedrock data (JSON) produced by the external [Extractor](https://github.com/Pumpkin-MC/Extractor) plus the build-time datapack extraction under `assets/datapack/`. See `assets/NOTICE.md` for attribution and licensing of third-party data.

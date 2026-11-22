# Responsibilities — what each module owns, and what it must not take on

> **Status:** Implemented — verified against crate layouts, `Cargo.toml` manifests and the repository's own agent contract ([AGENTS.md](../../AGENTS.md)) (2026-11-21).

Rules that cut across every crate:

- **Gameplay runs synchronously inside the tick.** `BlockBehaviour`, `ItemBehaviour` and `Goal` methods are sync. Networking, disk IO, plugins and events are async on Tokio; CPU-heavy work (chunk generation, lighting) goes to Rayon. No background task may mutate AI/world state after an entity's scheduled turn, and sync locks or `DashMap` guards are never held across `.await` (AGENTS.md, CONTRIBUTING.md "Working with Tokio and Rayon").
- **No hardcoded vanilla values.** Block/item properties, entity dimensions, tags, recipes, loot tables, sounds come from `pumpkin-data`, generated from `assets/`. Values vanilla hardcodes as `static final` Java constants are the only exception, kept as named Rust constants with the Java name.
- **Vanilla parity defines correctness.** A change is correct when it behaves like the official server (ordering, side effects, results), not when it compiles. Pumpkin's documented deviations — most importantly client-authoritative player movement — always win over a vanilla-faithful reading (AGENTS.md "Vanilla parity").

## Crate responsibilities and exclusions

### `crates/pumpkin` (binary)

- Owns: process startup, config file location resolution, logger init, panic → crash-report pipeline, OS signal handling, CLI flags (`--init-settings`, `--convert-world`), wiring plugin loaders into `PumpkinServer`.
- Must not: contain game logic, protocol logic or registration of new behaviours — that belongs in the crates below. If a new implementation needs registering, follow the wiring tables in AGENTS.md ("Where things get wired up") in `pumpkin-core`.

### `crates/pumpkin-core`

- Owns: server orchestration (`PumpkinServer`, tick loop), entities and their AI, block/item behaviour registries, command dispatch wiring, the networking layer above raw packets, plugin loading/permissions, entity islands (`world/entity_islands.rs`), crash reports, telemetry and update checking, logger bootstrap.
- Must not: know chunk file formats (that's `pumpkin-world`), define game data values (that's `pumpkin-data`), parse wire formats (that's `pumpkin-protocol`), or hold Mojang credentials/HTTP concerns beyond what `pumpkin-auth` exposes.

### `crates/pumpkin-config`

- Owns: the full `pumpkin.toml` schema (basic/advanced/telemetry), defaults, merge-with-defaults on load, rewrite when keys are missing.
- Must not: touch the network, the world on disk (beyond reading/writing its own TOML), or server state. Config is data, validated at load.

### `crates/pumpkin-protocol`

- Owns: Java and Bedrock wire formats, packet definitions, codec/serialization, the query protocol.
- Must not: make gameplay decisions or mutate world/entity state. The Java protocol targets exactly one Minecraft version at a time (`CURRENT_MC_VERSION` / `LOWEST_SUPPORTED_MC_VERSION` in `pumpkin-data`'s generated `packet.rs`); older clients are a plugin concern, not a protocol-crate concern.

### `crates/pumpkin-auth`

- Owns: HTTP session/registration clients and JWT validation for online-mode authentication.
- Must not: store or decide player state; it is a client boundary to Mojang services, not a player database.

### `crates/pumpkin-world`

- Owns: chunk storage formats (Anvil, Linear, Pump), chunk loading/saving pipelines, world generation, lighting engine, POI, dimension/biome definitions, world metadata, the region-format converter.
- Must not: depend on protocol or UI concepts; chunk conversion of player-visible behaviour happens in `pumpkin-core`. World generation differences from vanilla must be checked against vanilla source like any other gameplay change (AGENTS.md "Vanilla parity").

### `crates/pumpkin-data`

- Owns: the generated game data tables (`src/generated/` — codegen output only), curated data code around them, build-time datapack extraction from a vanilla server jar.
- Must not: contain behaviour logic, and must never be edited by hand under `src/generated/` (see [INV-0001](../plans/invariants/INV-0001-generated-data-not-hand-edited.md)).

### `crates/pumpkin-inventory`, `crates/pumpkin-command`, `crates/pumpkin-gametest`

- Own their gameplay domains: containers/screens, the command tree engine, the GameTest framework respectively.
- Must not: reach around each other or into networking directly; registration flows through `pumpkin-core` wiring points.

### Utility crates (`pumpkin-util`, `pumpkin-nbt`, `pumpkin-codecs`, `pumpkin-scheduler`)

- Own generic, game-agnostic machinery; depend only on each other in leaf positions.
- Must not: depend on any server crate, or grow gameplay-specific code.

### Plugin platform crates

- `pumpkin-plugin-wit`: the public API contract. Additive changes preferred; anything breaking breaks external SDKs too (mirrored repo). AGENTS.md requires running `cargo run --locked -p pumpkin-codegen -- wit` after WIT changes.
- `pumpkin-plugin-api`, `pumpkin-plugin-wit`, `pumpkin-plugin-utils`: **license boundary** — MIT OR Apache-2.0. Server (GPL-3.0) code must never move or be copied into these crates; see [INV-0005](../plans/invariants/INV-0005-plugin-crate-license-boundary.md).
- `pumpkin-wasm-host*`: execute plugins; versioned per API ABI (v0.1, v0.2) with shared code in `-common`. Hosts must change together with the WIT, the SDK and (when needed) `pumpkin-plugin-runtime`.
- Plugin platform crates must not be used to bypass server-side rules: plugin-visible events must keep firing at the same points with the same payloads, including cancellation semantics (AGENTS.md "Keep events working").

### Tools

- `pumpkin-codegen`: the only writer of `pumpkin-data/src/generated/`; must keep generation deterministic from `assets/`.
- `pumpkin-tester`: harness-only; headless bots and GameTests, not part of the shipped server.
- `pumpkin-fuzzer`: attack tool against a running server; relaxed lint rules are intentional and must not leak into server crates.

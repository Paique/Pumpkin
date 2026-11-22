# Flows — how the system runs end to end

> **Status:** Partial — entry points, configuration, threading model and wiring points are verified against `crates/pumpkin/src/main.rs`, `crates/pumpkin-core/src/lib.rs` and the config crate (2026-11-21). The packet-state-level detail of the Java login flow is summarized from the repository's own agent contract ([AGENTS.md](../../AGENTS.md)) and is **Unverified** line-by-line against the current protocol code.

## 1. Boot flow (`crates/pumpkin/src/main.rs`)

1. Tokio main starts; ring rustls provider installed before any HTTP client is built; global Rayon pool created with named `Rayon-Worker-N` threads.
2. Panic hook installed: every panic builds a `CrashReport` (saved to `./crash-reports/`), a main-thread panic aborts, a worker panic requests a graceful stop.
3. Config is loaded from the **current working directory**: `pumpkin.toml`, merged with defaults and rewritten when keys are missing (`pumpkin-config`). `--init-settings` / `--initSettings` exit(0) right after writing settings.
4. `VanillaData::load()` loads the game data produced from `assets/`.
5. `--convert-world <pump|anvil|linear>` converts the world folder to the target format and exits (uses `pumpkin-world`'s region converter).
6. Logger initialized from `advanced.logging` (level from `RUST_LOG` if set, else config); optional gzip rolling file logger; optional TTY console with command completion.
7. Plugin loaders are assembled (`WasmPluginLoader`, with config-controlled signature verification), `PumpkinServer::new(...).await` builds the server, `init_plugins()` runs (its wait time is excluded from the "Started server" timing), then `pumpkin_server.start().await` binds listeners and runs until shutdown. Exit code comes from `SERVER_EXIT_CODE`.

Logged at startup: server version, Java protocol version (`CURRENT_MC_VERSION` from generated `pumpkin-data`) and Bedrock version/protocol (`CURRENT_BEDROCK_MC_PROTOCOL` from `pumpkin-world`); debug builds warn that they must not be used for performance testing.

## 2. Threading model (applies to every flow)

- **Game logic — synchronous inside the tick.** Block, item and AI behaviour run on the tick; `BlockBehaviour`/`ItemBehaviour`/`Goal` methods are sync.
- **Networking, disk IO, plugins, events — async on Tokio.** Blocking a Tokio worker stalls ticks for every player.
- **CPU-heavy work — Rayon.** Chunk generation and lighting run in the Rayon pool; results cross back to Tokio through channels (the reference pattern is `Level::fetch_chunks`). Rayon calls from the Tokio runtime must be non-blocking (no `par_iter` on a Tokio worker).
- **Entity islands.** Independent entity groups tick in parallel; group formation is driven by reach reports (`Goal::tick_reach`, `Mob::has_audited_tick`) with a conservative fallback for unaudited mobs (see the [entity islands concept](../concepts/entity-islands-and-reach.md)).

## 3. Java client connection (status → login → play)

> Level of detail: **Unverified** against current code, summarized from AGENTS.md and README feature list. Confirmed anchors: status/ping, encryption, packet compression and proxy support are implemented; authentication goes through `pumpkin-auth` (reqwest/JWT) in online mode; proxy support covers BungeeCord, BungeeGuard and Velocity (`pumpkin-config/src/networking/proxy.rs`).

1. Client connects to the Java listener (default `0.0.0.0:25565`, configurable in `pumpkin.toml` under `networking.java`; can be disabled entirely).
2. Status/ping phase is answered without login state.
3. Login phase: compression and encryption are negotiated; in online mode the session is validated against Mojang services through `pumpkin-auth`.
4. Play phase: the server sends world state; the player's own movement is **client-authoritative** (a deliberate deviation from vanilla — see [ADR-0002](../plans/decisions/ADR-0002-client-authoritative-movement.md)).

## 4. Chunk pipeline

1. A chunk is requested through `pumpkin-world`'s chunk system (`chunk_system/`, `Level::fetch_chunks` channel pattern between Tokio and Rayon).
2. Generation and lighting run on Rayon; loaded chunks are cached and saved in the configured format — vanilla Anvil, Linear, or Pumpkin's Pump format (`chunk/io/`).
3. Worlds live in the folder named by `basic.default_level_name` (default `world`). Formats can be converted offline via `--convert-world`.
4. World time, borders, weather and ticking flow through `pumpkin-core`'s world coordination on top of the loaded chunks.

## 5. Plugin flow

1. At boot, `WasmPluginLoader` scans and loads Wasm plugins (signature verification per config `advanced.plugins.verify_signatures`).
2. Plugins execute in Wasmtime under the API version they target: host crates `pumpkin-wasm-host-v0_1` / `-v0_2` each generate their own bindings from the mirrored WIT; shared plumbing in `-common`. v0.2 requires building `pumpkin` with `--features plugin-v0_2`.
3. Plugin data folders live at `plugins/data/<name>` (permission model in `pumpkin-core/src/plugin/permissions.rs` gates reads/writes to the plugin's own folder).
4. Server events fire from `pumpkin-core`; cancellable events skip their effect when `cancelled()` returns true — existing event contracts must be preserved when porting gameplay (AGENTS.md "Keep events working").

## 6. Console commands

- With a TTY and `advanced.commands.use_tty`, the console uses rustyline with a `PumpkinCommandCompleter`; otherwise plain stdin lines are read.
- Console input goes through the same command dispatcher as in-game commands (`pumpkin-command`, registered in `pumpkin-core`'s `command/commands/`). `stop` shuts the server down.
- OS signals: Ctrl+C requests a stop; a second interrupt while already stopping forces exit. On Windows, Ctrl+Close/Shutdown are handled as termination signals (Ctrl+Logoff deliberately ignored).

## 7. Auxiliary listeners

Configured in `pumpkin.toml` under `networking`: server **query** (default `0.0.0.0:25565`), the Minecraft Server Management Protocol (MSMP), and LAN broadcast — each independently enabled/disabled.

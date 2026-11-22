# Plugin platform (Wasm)

How third-party code extends the server. Plugins are WebAssembly modules executed by Wasmtime against a **versioned WIT contract** that is public, mirrored to its own repository, and consumed by SDKs in other languages. Pumpkin cannot load Bukkit/Fabric plugins; this platform is its native extension mechanism. Decision: [ADR-0005](../plans/decisions/ADR-0005-wasm-plugin-platform-public-wit.md).

## Vocabulary

- **WIT** — the interface definition in `crates/pumpkin-plugin-wit/` (versions `v0.1/`, `v0.2/`); the public contract. Mirrored to `Pumpkin-MC/pumpkin-plugin-wit` on every push to `master`; not a Rust crate.
- **SDK** — `pumpkin-plugin-api` (MIT OR Apache-2.0); what plugin authors depend on. No internal Pumpkin dependencies.
- **Host** — the server-side executor: `pumpkin-wasm-host` (facade) → versioned hosts `-v0_1` / `-v0_2` (each generating its own bindings) over shared `-common` plumbing.
- **Runtime** — `pumpkin-plugin-runtime`: plugin lifecycle, executor, spawn policy.
- **Plugin data folder** — `plugins/data/<name>`; filesystem permissions are scoped per plugin (`pumpkin-core/src/plugin/permissions.rs`).

## Rules

- The WIT changes additively when possible; anything breaking must be called out because it breaks external SDKs. After WIT changes: update host + SDK (+ runtime when needed) together and run `cargo run --locked -p pumpkin-codegen -- wit`.
- The v0.2 ABI is unfinished and stays behind the `plugin-v0_2` feature of `pumpkin` (off by default).
- Breaking changes to the native plugin API bump `PLUGIN_API_VERSION` in `pumpkin-core`'s `plugin/mod.rs`.
- Event contracts are preserved: same fire points, same payloads, cancellation skips the effect — a port that silently skips an existing event breaks every plugin relying on it (AGENTS.md "Keep events working").
- License boundary: GPL server code never enters the permissive plugin crates ([INV-0005](../plans/invariants/INV-0005-plugin-crate-license-boundary.md)).

## Boundaries

This concept covers the **extension mechanism**, not the gameplay the server exposes through it. Plugin *authoring* documentation lives in `pumpkin-plugin-api`, not in the server docs. The platform does not grant plugins unscoped filesystem, network or world access — capabilities flow through the permission model and event system.

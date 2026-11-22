# ADR-0005: Plugins run as Wasm against a public, mirrored WIT contract

- **Status:** Accepted
- **Date:** 2026-11-21 (decision pre-dates this record; recorded by ddd-setup)
- **Deciders:** Pumpkin-MC maintainers (accepted before this record; documented in AGENTS.md "Plugin API")

## Context

Pumpkin cannot load Bukkit or Fabric plugins (it is not a fork of the Java server). The plugin surface must therefore be defined from scratch, usable by external SDKs in other languages, and stable enough for plugin authors to depend on. The repository implements it as: WIT packages in `crates/pumpkin-plugin-wit` (API versions v0.1 and v0.2), permissively licensed SDK crates (`pumpkin-plugin-api` under MIT OR Apache-2.0, unlike the GPL-3.0 server), Wasmtime-based hosts (`pumpkin-wasm-host*`) that each generate their own bindings, and CI that subtree-mirrors the WIT directory to `Pumpkin-MC/pumpkin-plugin-wit` on every push to `master` (`sync-wit.yml`).

## Decision

1. **The WIT is the public contract.** It is mirrored to its own repository and SDK bindings for other languages are generated from it — a breaking change there breaks SDKs outside this repo. Prefer additive changes; call out anything breaking.
2. **Layers change together:** a WIT change requires the matching host implementation (`pumpkin-wasm-host-v0_1` / `-v0_2`), the SDK (`pumpkin-plugin-api`) and sometimes `pumpkin-plugin-runtime`, followed by `cargo run --locked -p pumpkin-codegen -- wit` (generated WIT data and packet mappings).
3. **ABI versions are physical crates.** Each API version has its own host crate generating its own bindings; the unfinished v0.2 ABI is gated behind the `plugin-v0_2` feature of `pumpkin`, off by default until the ABI is done. Breaking changes to the native plugin API bump `PLUGIN_API_VERSION` in `pumpkin-core`'s `plugin/mod.rs`.
4. **License boundary:** server (GPL-3.0) code never moves into the permissively licensed plugin crates ([INV-0005](../invariants/INV-0005-plugin-crate-license-boundary.md)).
5. **Event contracts are sacred:** when porting gameplay a plugin could observe or cancel, existing events keep firing at the same points with the same payloads; cancellable events skip their effect when cancelled.

## Alternatives considered

- **Bukkit/Fabric-compatible plugin loading** — rejected: those ecosystems are Java-server-bound; Pumpkin has no compatible runtime to host them.
- **Native dynamic libraries as the only plugin format** — rejected for the public contract: Wasm gives sandboxing, cross-language bindings and a versioned interface file (WIT) that native ABIs lack. (Native in-process APIs still exist internally, with `PLUGIN_API_VERSION` bumping on breaks.)
- **Single unversioned plugin API** — rejected: host crates are already split by API version precisely so old plugins keep working while new ABIs evolve.

## Consequences

- Plugin-platform changes are cross-cutting (WIT + host + SDK + runtime + codegen) and get extra CI (`check_wit` validates both WIT packages with `wit-bindgen`/`wasm-tools`).
- v0.2 stays behind a feature flag until its ABI is finished; server builds and tests run without it unless the flag is passed.
- Plugin authors target `pumpkin-plugin-api` (permissive license) — documentation for *writing* plugins lives there, not in the server docs.

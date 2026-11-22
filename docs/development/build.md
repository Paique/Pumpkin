# Building

> **Status:** Implemented — verified against workspace `Cargo.toml`, CI workflows (`.github/workflows/rust.yml`, `docker.yml`, `nightly.yml`, `release.yml`, `nix.yml`) and `crates/pumpkin/src/main.rs` (2026-11-21).

## Workspace

One Cargo workspace: 22 crates in `crates/` plus 3 tools in `tools/` (see [modules.md](../architecture/modules.md)). `Cargo.lock` is committed; CI builds with `--locked` where release/deploy paths require it.

```sh
cargo build                       # debug — default for iteration
cargo build --release             # optimized; slow due to LTO settings (see below)
```

Build-time note: the first build of `pumpkin-data` fetches the vanilla server jar for datapack extraction unless `PUMPKIN_MINECRAFT_SERVER_JAR` is set ([setup.md](setup.md)).

## Release profile

- The shipped nightly/release binaries use the release profile **as configured in `Cargo.toml` (fat LTO)** — slow to build, fastest to run.
- CI push builds default to **LTO off + 16 codegen units** to stay green and quick; manual `workflow_dispatch` runs can override `release_lto` (off/thin/fat) and `release_codegen_units`.
- Practical rule from AGENTS.md: use **debug builds** while developing; reserve release builds for performance comparison runs (same machine, same scenario, ideally against `master` for before/after numbers).

## Features

| Feature | Crate | Default | Purpose |
| --- | --- | --- | --- |
| `plugin-v0_2` | `pumpkin` | **off** | Enables the v0.2 Wasm plugin host ABI (still unfinished; keep off unless working on it). Build with `--features plugin-v0_2`. |
| `console-subscriber` | `pumpkin` | off | Initializes `tokio-console`'s subscriber at startup ([debug.md](debug.md)). |

## Code generation

```sh
cargo run --locked -p pumpkin-codegen          # regenerate pumpkin-data/src/generated from assets/
cargo run --locked -p pumpkin-codegen -- wit   # regenerate WIT-derived data and packet mappings
```

Never edit `crates/pumpkin-data/src/generated/` by hand ([INV-0001](../plans/invariants/INV-0001-generated-data-not-hand-edited.md)); details and the assets pipeline are in `tools/pumpkin-codegen/AGENTS.md`. After WIT changes, the WIT, host crates (`pumpkin-wasm-host-v0_1` / `-v0_2`), the SDK (`pumpkin-plugin-api`) and sometimes `pumpkin-plugin-runtime` must change together (AGENTS.md "Change the layers together").

## Platform matrix (from CI)

| Target | Built where | Notes |
| --- | --- | --- |
| Linux x64, Linux ARM64, Windows x64, Windows ARM64, macOS ARM64 | `rust.yml` | PR builds (affected packages) + full release builds on push. Docker images also ship an `X64-v3` x86-64-v3 variant. |
| Android aarch64 | `rust.yml` (`cargo ndk -t arm64-v8a`), NDK r30, `CARGO_NDK_PLATFORM=24` | Tests run on an API-35 emulator; test assets are pushed to `/data/local/tmp/assets` first. |
| WASI | — | Compile-time error; not supported. |
| Nix | `flake.nix` / `default.nix`, CI `nix.yml` | Alternative build/packaging path. |

## Packaging

- CI publishes: canary release artifacts per push to `master`, a nightly build with the full release profile, tagged releases (`release.yml`), and Docker images (`docker.yml`). These run on the upstream repository's infrastructure; on a fork they will run only if actions are enabled with the same secrets.

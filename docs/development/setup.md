# Development setup

> **Status:** Implemented — verified against `rust-toolchain.toml`, `Cargo.toml`, CI workflows and `crates/pumpkin-data/build.rs` (2026-11-21).

## Toolchain

- **Rust stable** — pinned by `rust-toolchain.toml` (channel `stable`, components `rust-analyzer` + `rust-src`). A plain `rustup` install picks it up automatically on first `cargo` command inside the repo.
- **Python 3** — only needed to run the CI package planner locally (`.github/scripts/ci_packages.py`); not needed for everyday builds.
- Rust 2024 edition (`rustfmt.toml`); WASI targets are **unsupported** (compile error in `crates/pumpkin/src/main.rs`).

## Optional tools used by the repo's own workflow ([AGENTS.md](../../AGENTS.md))

```sh
cargo install cargo-nextest --locked   # test runner used by CI
cargo install typos-cli --locked       # spellcheck (`typos` command)
```

For plugin-API (WIT) work only: `wit-bindgen` and `wasm-tools` (CI installs them via the `bytecodealliance` setup actions in `check_wit`).
For Android tests only: Android NDK r30 + `cargo install cargo-ndk` + `rustup target add aarch64-linux-android`.

## First build — network requirement

`crates/pumpkin-data` has a `build.rs` that extracts the vanilla datapack: it **downloads the Minecraft server jar from Mojang's piston-meta manifest at build time**, unless you point `PUMPKIN_MINECRAFT_SERVER_JAR` at a local server jar of the right version first:

```sh
export PUMPKIN_MINECRAFT_SERVER_JAR=/path/to/server.jar   # optional offline shortcut
cargo build
```

Expect `cargo:warning=Extracting datapack from server jar...` lines during the first `pumpkin-data` build; the result is materialized under `assets/datapack/`.

## Platform support

CI builds and tests on: Linux x64/ARM64, Windows x64/ARM64, macOS ARM64, and Android aarch64 (emulator test job). See [build.md](build.md) for the platform matrix and release profiles.

## Where to go next

- Running the server: [run.md](run.md)
- Building (profiles, features, codegen): [build.md](build.md)
- Tests: [test.md](test.md) — Lints: [lint.md](lint.md) — Formatting: [formatting.md](formatting.md)
- Debugging: [debug.md](debug.md)
- Repository rules for agents: the root [AGENTS.md](../../AGENTS.md) (authoritative; it also links area-specific AGENTS.md files in `crates/pumpkin-protocol/`, `crates/pumpkin-world/` and `tools/pumpkin-codegen/`).

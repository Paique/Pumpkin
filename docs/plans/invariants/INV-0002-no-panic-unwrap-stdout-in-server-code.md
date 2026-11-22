# INV-0002: Server code never unwraps, expects, panics, todos or prints to std streams

- **Status:** Enforced
- **Scope:** all workspace crates and tools, except where a scoped exception is explicit

## Statement

Server code contains no `unwrap()`, `expect()`, `panic!`, `todo!`, and no printing to stdout/stderr — including in error paths. `Result`/`Option` are handled for real; diagnostics go through `tracing` logging.

## Rationale

A server that panics on reachable input takes down every player's session (panics become crash reports and force shutdown — see the panic hook in `crates/pumpkin/src/main.rs`). `unwrap`/`expect` are exactly how panics slip into "can't happen" paths, and direct stdout/stderr printing corrupts the console (which doubles as a command interface) and bypasses log formatting/filtering.

## Enforcement

- `[workspace.lints.clippy]` denies `unwrap_used`, `expect_used`, `panic`, `todo`, `print_stdout`, `print_stderr` (plus the `all`/`pedantic`/`nursery`/`cargo` groups) in the workspace `Cargo.toml`.
- `crates/pumpkin/src/main.rs` adds a crate-level `#![deny(clippy::unwrap_used)]`.
- CI sets `RUSTFLAGS="-Dwarnings"` and runs `cargo clippy --all-targets --all-features` on affected packages ([lint.md](../../../docs/development/lint.md)), so violations fail the build.
- The sanctioned exception is a **local** `#[expect(..., reason = "...")]` stating why (AGENTS.md "Code quality"); test builds may re-allow unwrap/expect/panic via `cfg_attr(test)` (pattern in `main.rs`). `tools/pumpkin-fuzzer` deliberately relaxes these for itself.

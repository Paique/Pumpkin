# Testing

> **Status:** Implemented — commands verified against `.github/workflows/rust.yml`, `tools/pumpkin-tester/`, `crates/pumpkin-gametest/` and bench targets in crate manifests (2026-11-21).

## Unit and integration tests

```sh
cargo nextest run -p <crate> --no-tests=pass   # what CI and AGENTS.md use
cargo test -p <crate>                          # plain cargo also works
cargo test --doc -p <crate>                    # documentation tests (CI runs these separately)
```

CI does not run the whole workspace on every PR: `.github/scripts/ci_packages.py` computes the packages affected by the diff and runs `cargo nextest run --verbose --no-tests=pass` plus `cargo test --doc` on those. Locally, run the crates you touched **plus the crates that depend on them**.

Scope-wide test jobs also exist for: release-tagged full runs, and Android (emulator, `cargo ndk -t arm64-v8a test --workspace` after pushing `assets/` to the device).

## Test harness (bots + GameTests)

```sh
cargo run -p pumpkin-tester
```

`tools/pumpkin-tester` boots the server harness and runs:
- **Headless protocol bots** (`bot.rs`, `bot_tests/`) that join and exercise gameplay over the real protocol;
- **Mojang GameTests** through the framework implemented in `crates/pumpkin-gametest` (`gametests.rs`).

Behaviour that needs a real world belongs here, not in unit tests.

## Benchmarks

Criterion benches exist in: `pumpkin-data`, `pumpkin-nbt`, `pumpkin-util`, `pumpkin-world` (see each crate's `Cargo.toml` `[[bench]]`). Run with `cargo bench -p <crate>`. For performance work, run the same benches on both commits before chasing a slowdown (AGENTS.md "Performance").

## Network fuzzing

`tools/pumpkin-fuzzer` is a clap-based packet fuzzer/stress tool (modes: `all`, `raw`, `framed`, `stateful`, `corrupt-varint`) aimed at a running server. See its `--help` for the exact flag set:

```sh
cargo run -p pumpkin-fuzzer -- --help
```

## What deserves a test (house rules)

AGENTS.md "Tests" is authoritative — short version: write tests for codecs/NBT/parsers (round trips vs. real vanilla bytes), quietly-breakable logic (scheduling, caching, ordering, boundaries), bug regressions (test fails without the fix), and comparisons against real vanilla output. Do **not** write tests that repeat the implementation, check generated tables against themselves, cover constructors/derives, or mock away everything real. When a change can't be tested meaningfully in code, say so and describe the boot, bot or in-game check instead.

## What needs a real client

Rendering, particles, sounds, animations, GUI screens and anything involving client-side prediction cannot be verified by bots or unit tests. For such changes the author joins with a real client; bots (Mineflayer, rust-mc-bot) cover movement/chunk/inventory/packet-level checks ([debug.md](debug.md)).

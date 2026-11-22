# Debugging

> **Status:** Implemented — verified against `crates/pumpkin-core/src/lib.rs` (logger), `crates/pumpkin-core/src/crash.rs`, `crates/pumpkin/src/main.rs` (panic hook, feature flag) and AGENTS.md guidance (2026-11-21).

## Logging

- Framework: `tracing` + `tracing-subscriber`, initialized in `pumpkin_core::init_logger` from `pumpkin.toml` → `[advanced.logging]`.
- Level: `RUST_LOG` env var wins; otherwise `advanced.logging.level`. A full `EnvFilter` directive string in `RUST_LOG` is honored (`EnvFilter::try_from_default_env`).
- Console formatting switches under `[advanced.logging]`: `color`, `threads`, `thread_ids`, `target`, `timestamp`; file logging via `advanced.logging.file` (gzip rolling logger; empty string disables).
- House rule (AGENTS.md): log with `tracing`, never print to stdout/stderr (clippy denies `print_stdout`/`print_stderr`); admin-facing messages are one short line saying what's wrong.

## Crash reports

The panic hook (`crates/pumpkin/src/main.rs`) captures a backtrace on every panic and saves a crash report to **`./crash-reports/`** (relative to the working directory — see `CrashReport::save` in `crates/pumpkin-core/src/crash.rs`). A main-thread panic aborts the process; a worker panic triggers a graceful shutdown with the report logged. Read these first when a run died.

## Boot test (the baseline loop)

Start the server in a scratch directory, wait for the "done" log line, drive the feature via console commands, stop with `stop`, then grep the log for panics/errors/warnings your change added. Full procedure and the do/don't list live in AGENTS.md "Running the server yourself" — it is authoritative, don't skip it.

## Bots (headless clients)

- [Mineflayer](https://github.com/PrismarineJS/mineflayer) — scriptable; good for "walk here, use item, check block" scenarios.
- [rust-mc-bot](https://github.com/Eoghanmc22/rust-mc-bot) — many-player load scenarios.
- Bot libraries often lag the newest protocol — check the supported version before blaming the server. What still needs a real client: rendering, particles, sounds, animations, GUIs, client-prediction-dependent behaviour.

## Performance work

- Debug builds warn at startup that they are unfit for performance measurement — use `--release` for timing runs (and expect the fat-LTO build to be slow to produce).
- For load/memory comparisons: same seed, bot count, view distance, duration on both `master` and your branch; record MSPT / peak RSS; one run per side isn't enough when numbers are close (AGENTS.md).
- Chasing a bug: add temporary counters or `tracing` output to **prove** the cause before changing code, then remove the instrumentation. Keep investigation scripts out of the repo.
- The repository's benches (criterion, see [test.md](test.md)) are the first stop when a slowdown appears — the cause is often not where it looks.

## Tokio-level inspection

Building the binary with `--features console-subscriber` initializes `tokio-console`'s subscriber at startup (`main.rs`) — useful for spotting tasks blocking workers. Remember the model: gameplay sync in tick, IO on Tokio, CPU work on Rayon; blocking a Tokio worker stalls every player's ticks.

## Network robustness

Aim `tools/pumpkin-fuzzer` (raw/framed/stateful/corrupt-varint modes) at a running server to stress packet parsing — malformed input handling must bound lengths and allocations (AGENTS.md), and the fuzzer is the cheap way to find out whether it does.

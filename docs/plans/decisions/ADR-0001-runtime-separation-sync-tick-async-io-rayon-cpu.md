# ADR-0001: Runtime separation — sync tick, async Tokio, CPU work on Rayon

- **Status:** Accepted
- **Date:** 2026-11-21 (decision pre-dates this record; recorded by ddd-setup)
- **Deciders:** Pumpkin-MC maintainers (accepted before this record)

## Context

A Minecraft server mixes three workloads with opposite requirements: gameplay logic needs deterministic ordering (ticks), networking and disk IO need concurrency, and chunk generation/lighting need raw CPU parallelism. Blocking one runtime stalls ticks for every connected player. The threading contract is stated in [AGENTS.md](../../../AGENTS.md) ("Game logic … runs synchronously inside the tick"), in [CONTRIBUTING.md](../../../CONTRIBUTING.md) ("Working with Tokio and Rayon") and enforced by review; `crates/pumpkin/src/main.rs` carries the warning comment that Rayon calls from the Tokio runtime must be non-blocking.

## Decision

Three execution domains with a fixed crossing pattern:

1. **Game logic runs synchronously inside the tick** — `BlockBehaviour`, `ItemBehaviour` and `Goal` methods are sync functions on the tick thread(s).
2. **Networking, disk IO, plugins and events are async on Tokio.** Sync locks / `DashMap` guards are never held across `.await`.
3. **CPU-heavy work (chunk generation, lighting) runs on Rayon**, and results cross to Tokio through channels — the canonical pattern is `Level::fetch_chunks`. Never call blocking Rayon entry points (e.g. `par_iter`) from a Tokio worker.

Alternatives considered and implicitly rejected by this design: running gameplay on async tasks (loses tick ordering guarantees), and doing chunk work inline on Tokio workers (stalls all players). Entity-level parallelism is layered on top via entity islands ([ADR-0004](ADR-0004-entity-islands-with-reach-reporting.md)).

## Consequences

- New AI or block code must not spawn background tasks that mutate AI/world state after its scheduled turn (CONTRIBUTING "State ownership and queries").
- Debugging stalls means inspecting three pools (Tokio workers, Rayon workers, tick); `--features console-subscriber` exists for the Tokio side.
- New IO or CPU-bound features must pick their lane explicitly; code review rejects lanes that block the wrong runtime.

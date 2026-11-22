# ADR-0004: Entity islands tick in parallel, bounded by reach reports

- **Status:** Accepted
- **Date:** 2026-11-21 (decision pre-dates this record; recorded by ddd-setup)
- **Deciders:** Pumpkin-MC maintainers (accepted before this record; documented in CONTRIBUTING "Entity island reach" and AGENTS.md "Mob AI and entity islands")

## Context

The server wants mobs ticked in parallel for performance, but entity AI reads and writes shared world state. Ticking two entities that interact on different threads breaks ordering; merging every entity into one group forfeits the parallelism. The implementation is in `crates/pumpkin-core/src/world/entity_islands.rs`: independent groups ("islands") of entities tick in parallel, entities within a group tick in list order. Group formation needs **bounds** on what each entity can read or change during its turn — a missing interaction can put dependent entities on different threads, and over-reporting merges groups that could have run independently.

## Decision

Every entity's per-tick access is declared up front:

- Goals implement `Goal::tick_reach()` returning a `GoalReach` — distances within which the goal may search/read/affect entities and players, the highest navigation speed modifier it may set, whether it may change blocks, and entity/player IDs reachable regardless of distance (`links`).
- Species-level access is summarized by `Mob::has_audited_tick()` + `MobEntity::audited_reach`, set **true only after auditing** the species' tick, custom AI and Brain behaviour against those bounds.
- Mobs without audited bounds use a **conservative fallback** (finite margins). An unaudited goal returns `None` from `tick_reach`. An *empty* `GoalReach` means "no additional access", never "unknown access".
- Reach reporting must not itself consume randomness, select targets, advance timers, invoke gameplay callbacks or mutate the world.
- Effects outside the model's bounds (long-range explosions, teleports, chained block updates) require explicit dependency/global reach handling; `changes_blocks` alone never describes a distant write location.

## Alternatives considered

- **Single-threaded ticking** — rejected: forfeits the multi-threading goal stated in the README.
- **Lock-per-entity / global lock** — rejected: lock contention serializes anyway and deadlocks are hard to rule out with AI graphs this dynamic.
- **Optimistic concurrency with retries** — rejected: AI interactions are not idempotent (randomness, cooldowns, sounds), so replaying a tick turn is not safe.

## Consequences

- New AI must preserve goal priorities, lifecycle, cadence and reach semantics — and validate lifecycle transitions with focused regressions (CONTRIBUTING "Validation").
- Reach bounds are a correctness contract, not documentation: an incomplete report can change interaction order, so audits are re-checked whenever behaviour changes.
- Details and house rules live in [CONTRIBUTING.md](../../../CONTRIBUTING.md) and AGENTS.md; see also the [entity islands concept](../../concepts/entity-islands-and-reach.md).

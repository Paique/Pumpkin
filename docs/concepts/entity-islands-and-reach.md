# Entity islands and reach

How the server ticks many entities in parallel without breaking interaction order. The world's entities are partitioned into independent groups ("islands") that tick concurrently; entities **within** an island tick in list order, so anything that can interact stays sequentially ordered. Island formation is computed from declared per-tick access bounds, not from runtime locks. Implementation: `crates/pumpkin-core/src/world/entity_islands.rs`; contract: [ADR-0004](../plans/decisions/ADR-0004-entity-islands-with-reach-reporting.md).

## Vocabulary

- **Island** — a group of entities that may interact with each other and therefore tick together, in order, on one lane.
- **Reach** — the declared bound on what one entity (or one AI goal within one tick) may search, read, affect or retain references to.
- **`GoalReach`** — a goal's report for the upcoming tick: entity/player search distances, max navigation speed it may request, whether it may change blocks (`changes_blocks`, which does **not** locate distant writes), and distance-independent `links` to specific entity/player IDs.
- **Audited reach** (`Mob::has_audited_tick()` + `MobEntity::audited_reach`) — species-level bounds derived from reviewed behaviour (own tick, custom AI, Brain callbacks, movement, block access, spawns, remote effects).
- **Conservative fallback** — the finite, broader margins used for any mob without audited bounds or whose goal returns `None` from `tick_reach()`.

## Rules

- An **empty** report means "no additional access" — it is never a synonym for "unknown access". An **unaudited** goal returns `None` and drops the mob to the fallback.
- Reach computation must not consume randomness, select targets, advance timers, invoke goal callbacks or mutate the world.
- `has_audited_tick()` flips to true only after auditing the species' whole behaviour, and is revisited whenever that behaviour changes.
- Effects outside all modelled bounds (long-range explosions, teleports, chained block updates) need explicit dependency/global reach handling; `changes_blocks = true` alone never covers arbitrary distant writes.
- Entity queries keep their exact filtering, player inclusion and result order — unordered iteration can change which target wins a tie. Entities move only through position/pose/size/index-aware APIs so the spatial index stays consistent.
- Coverage of these rules: CONTRIBUTING.md "Entity island reach" and AGENTS.md "Mob AI and entity islands" are the authoritative texts; the lifecycle/reach regressions required there are the check layer.

## Boundaries

Entity islands parallelize **ticking**, not networking, chunk generation or lighting (those follow the runtime model, [ADR-0001](../plans/decisions/ADR-0001-runtime-separation-sync-tick-async-io-rayon-cpu.md)). They are not a general ECS scheduler: no background tasks may mutate AI state after an entity's turn, and no alternative AI scheduler/movement loop may be added. "Island" here has nothing to do with world islands or biome islands in game terms.

# ADR-0002: Player movement is client-authoritative

- **Status:** Accepted
- **Date:** 2026-11-21 (decision pre-dates this record; recorded by ddd-setup)
- **Deciders:** Pumpkin-MC maintainers (accepted before this record)

## Context

The vanilla Java server simulates and validates the player's own movement server-side. Pumpkin states in [AGENTS.md](../../../AGENTS.md) ("Vanilla parity") that it deliberately differs from vanilla in a few places, and that "the player's own movement is client authoritative" — the most important of those deviations. Any change ported faithfully from the vanilla source can still break this assumption, which is why the parity rule explicitly defers to documented deviations.

## Decision

The player's own movement is trusted from the client: the server applies the client-reported position/rotation instead of running a server-authoritative simulation of the owning player's movement. Server-side checks still govern everything the client does not own (collisions of other entities, teleports initiated by the server, knockback and other external forces).

This deviation is part of the definition of "correct" in this codebase: vanilla parity means matching observable behaviour **except** where a documented deviation overrides it. When porting code, check how Pumpkin already handles the touched area (AGENTS.md "Porting", final bullet of the section).

## Alternatives considered

- **Full server-authoritative movement like vanilla** — rejected: it costs server CPU on the hottest path (every player moves every tick) and adds latency for the player; performance is a stated project goal. Anti-cheat hardening could revisit this, which would be a new ADR superseding this one.
- **Per-case hybrid validation** — rejected for now: each deviation from client authority needs its own port and maintenance burden; there is no evidence in the repo of such per-case checks being implemented.

## Consequences

- Sending the server's copy of a player's state back to that same player fights client-side prediction — check who vanilla actually sends each update to before porting (AGENTS.md "Porting"; CONTRIBUTING "Know what the client simulates" in AGENTS.md).
- Movement-affecting gameplay ports (knockback, vehicle riding, teleport effects) must only touch the parts of movement the server owns.
- Bots and unit tests cannot fully verify movement feel; real-client checks are required for movement changes (AGENTS.md "Running the server yourself").

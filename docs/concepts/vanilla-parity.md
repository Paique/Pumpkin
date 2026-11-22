# Vanilla parity

Pumpkin's definition of correctness: a gameplay change is correct when the game **behaves** like the official (vanilla) Java Edition server — same ordering, same side effects, same observable results — not when it compiles or matches a wiki description. Vanilla defines behaviour and outcomes, never a required internal algorithm; a different implementation is fine when the observable result is equivalent.

## Vocabulary

- **Vanilla** — the official Java Edition server; the behavioural source of truth.
- **Port** — implementing a vanilla behaviour in Rust, traced from the decompiled Java entry point through the full call chain (superclasses and overrides included).
- **Documented deviation** — a deliberate, recorded difference from vanilla that overrides parity. The canonical example is client-authoritative player movement ([ADR-0002](../plans/decisions/ADR-0002-client-authoritative-movement.md)).

## Rules

- Port from the decompiled vanilla source, never from memory or the wiki — both are often wrong about tick order, rounding and side-effect placement (AGENTS.md "Vanilla parity" gives the source-download procedure via piston-meta + Vineflower).
- A port must carry implicit behaviour explicitly (e.g. vanilla loading a chunk as a side effect of a block read) and preserve early-return ordering (stats, cooldowns, events, sounds happening before a cancel).
- Values, flags and enums are **translated** to their Pumpkin equivalents, not copied numerically — except vanilla `static final` constants, kept as named Rust constants with the Java name ([ADR-0003](../plans/decisions/ADR-0003-generated-game-data-pipeline.md)).
- Parts of a port that couldn't be checked against vanilla must be declared in the PR description.

## Boundaries

Vanilla parity does **not** mean reimplementing vanilla's internal design, keeping Java class structure, or refusing Pumpkin-idiomatic Rust. It does not extend to Bedrock Edition, which has no vanilla-parity baseline documented yet ([GAP-0002](../plans/gaps/GAP-0002-bedrock-support-undocumented.md)). It never overrides a documented deviation.

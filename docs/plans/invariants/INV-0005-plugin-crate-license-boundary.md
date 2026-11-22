# INV-0005: GPL server code never moves into permissively licensed plugin crates

- **Status:** Documented-only
- **Scope:** `crates/pumpkin-plugin-api`, `crates/pumpkin-plugin-wit`, `crates/pumpkin-plugin-utils` (MIT OR Apache-2.0) vs. the GPL-3.0 server crates

## Statement

No code from the GPL-3.0 server crates is moved, copied or paraphrased into `pumpkin-plugin-api`, `pumpkin-plugin-wit` or `pumpkin-plugin-utils`. Those three crates stay entirely license-clean so plugin authors can depend on them under MIT OR Apache-2.0.

## Rationale

The dual licensing is deliberate ([ADR-0005](../decisions/ADR-0005-wasm-plugin-platform-public-wit.md)): plugin authors must be able to write and distribute plugins without their work being forced into GPL by a dependency. If server code leaks into these crates, the whole permissive surface becomes legally ambiguous — or effectively GPL — and external SDK projects inherit the contamination. AGENTS.md states it as "Don't move or copy server code into those crates."

## Enforcement

Currently **review-only**: the rule lives in AGENTS.md "Plugin API" and in the license files inside `crates/pumpkin-plugin-api/` and `crates/pumpkin-plugin-wit/`. No automated license-boundary check exists. Cheapest enforcement point: a CI guard that fails when a permissively licensed crate adds a dependency on any GPL server crate (trivially checkable from manifests — today none of the three has internal dependencies), plus periodic review for copied code.

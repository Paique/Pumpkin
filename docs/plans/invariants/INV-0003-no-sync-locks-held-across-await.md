# INV-0003: Synchronous locks are never held across `.await`

- **Status:** Documented-only
- **Scope:** all async code in the workspace (Tokio tasks, handlers, network paths)

## Statement

No `std::sync` mutex/RwLock guard, `DashMap` reference guard or other blocking lock is held across an `.await` point. Data that must survive an await is cloned or the lock is reacquired after the await.

## Rationale

Holding a sync lock across an await can block the Tokio worker thread or deadlock the runtime when another task holding the lock awaits the same worker — and a blocked Tokio worker stalls ticks for **every** player ([ADR-0001](../decisions/ADR-0001-runtime-separation-sync-tick-async-io-rayon-cpu.md)). The rule is stated in AGENTS.md ("Don't block Tokio workers… sync locks or `DashMap` guards are never held across `.await`") and CONTRIBUTING.md ("Never hold synchronous guards across `.await`", including the navigator/Brain re-entrancy rules).

## Enforcement

Currently **review-only** plus whatever the default clippy groups catch incidentally; the repo does not enable a lint dedicated to this pattern (e.g. `clippy::await_holding_lock`) explicitly in `[workspace.lints]`. Cheapest enforcement point: add `await_holding_lock = "deny"` to `[workspace.lints.clippy]` and let CI's `RUSTFLAGS="-Dwarnings"` + clippy job fail on violations. Note the re-entrancy variants (calling a helper that locks the same navigator/brain while holding its guard) are *not* catchable by any existing lint and remain review-enforced.

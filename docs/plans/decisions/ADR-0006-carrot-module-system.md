# ADR-0006: Carrot features as runtime-toggleable native modules

- **Status:** Accepted
- **Date:** 2026-11-21
- **Deciders:** Latte Studios (carrot fork)

## Context

The carrot server must reproduce the behavior of the Latte Studios build server, which on Paper/Purpur is delivered by ~34 plugins (LuckPerms groups, a creative-by-permission Skript, FAWE and a stack of terraforming addons, chat channels, multiworld). Carrot cannot load any of them (see [ADR-0005](ADR-0005-wasm-plugin-platform-public-wit.md)), so every feature is reimplemented natively in Rust. The reference server's behavior, plugin inventory and permission layout are catalogued in `carrot-server/claudedocs/research_carrot-module-port-buildserver_2026-11-21.md`.

Requirements:

- Features must be **enableable/disableable at runtime**, like plugins on Paper — not compile-time cargo features (which upstream's `AGENTS.md` discourages for hypothetical use anyway).
- The diff against upstream Pumpkin must stay minimal so rebasing stays cheap.
- Modules must not depend on unstable plugin loading (`libloading` native plugins are ABI-locked to an exact server build; Wasm plugins are sandboxed away from the world-editing internals future modules need).

## Decision

1. **Modules are workspace crates compiled into the server binary.** `crates/carrot-core` provides the module system (`CarrotModule` trait, `ModuleContext`, `Carrot` manager); each feature lives in its own crate (aggregated by `crates/carrot`). The only upstream-tree change is the `carrot::init` call in `crates/pumpkin/src/main.rs` plus workspace/member and dependency entries.
2. **Enable/disable reuses the plugin machinery.** Every module gets a synthetic plugin `Context` (metadata `carrot-<id>`), so commands, event handlers and permissions register exactly like a plugin's: `Context::register_command` (atomic dispatcher swap + command tree re-broadcast) and `Context::register_event` (source-tagged handlers). Disabling a module removes its handlers by source from the shared `HandlerMap` and unregisters its tracked command names — the same semantics as plugin unload.
3. **State lives in `<exec_dir>/carrot/`:** `carrot.toml` (per-module enabled map, materialized with each module's default on boot), `groups.toml` (LuckPerms-style groups with inheritance, players by name, `-node` denies; seeded from the reference server's `default`/`builder` layout), and `modules/<id>/` data folders.
4. **Permissions use the `carrot:` namespace** (`carrot:admin`, `carrot:buildermode`, …) so the `PermissionManager`'s existing wildcard resolution (`carrot:*`, `carrot:module.*`) applies to groups unchanged. Group permissions are applied to a player's attachment on join (blocking handler, registered before any module handler) and re-applied via `/carrotgroups reload`.
5. **`/carrot module list|enable|disable`** manages modules at runtime; `/carrotgroups reload` reloads groups. Both are root infrastructure (always registered) so a disabled module can always be re-enabled.
6. **Ordering contract:** the group-join handler and modules' join handlers are registered blocking, groups first — blocking handlers run sequentially in registration order, so a module may trust that group permissions are already applied when its join handler runs.

## Alternatives considered

- **One patch stack inside `pumpkin-core`** (Purpur-style): rejected — maximizes rebase pain, no runtime toggling.
- **Native `libloading` plugins** (`PLUGIN_API_VERSION = 6`): rejected as the primary vehicle — a single version integer means the `.dll` must be built against the exact server build, and the plugin `Context` does not expose the world internals a WorldEdit module needs. Kept available for third parties.
- **Wasm plugins:** rejected for first-party features — sandboxing blocks deep world access; remains the supported public platform (ADR-0005).
- **Compile-time cargo features per module:** rejected — violates the runtime-toggle requirement and upstream's "keep it as small as the task" guidance.

## Consequences

- Upstream syncs stay cheap: carrot code lives only in `crates/carrot*` + one `init` call.
- First module delivered with this ADR: `builder_mode` (parity with the reference server's `criativo.sk` — `carrot:buildermode` switches a player to creative on join/respawn, others back to survival).
- Future modules (essentials, multiverse, chat, worldedit engine, terrain) plug in via `register_module` in `crates/carrot/src/lib.rs`; none of them may modify the upstream tree.
- Permission nodes registered by a disabled module stay in the registry (harmless: nothing checks them while the module is off); handlers and commands are fully removed.
- `on_enable`/`on_disable` hooks are synchronous by design; modules spawn async work onto tokio when needed, following upstream's async rules.

# Dependencies — internal direction and external systems

> **Status:** Implemented — internal edges extracted from every crate's `Cargo.toml` `[dependencies]` section; external systems from `build.rs`, `crates/pumpkin-auth/src/`, `Dockerfile` and workflow files (2026-11-21).

## Allowed internal direction

The rule of thumb: **leaves → domains → core → binary**. Nothing depends *up* the stack.

```
pumpkin (binary)
 └─ pumpkin-core ──► protocol, auth, config, data, world, inventory, command, macros, nbt, util, gametest
      │
      ├─ pumpkin-world ──► config, data, nbt, util
      ├─ pumpkin-protocol ──► data, macros, nbt, util
      ├─ pumpkin-command ──► codecs, data, nbt, protocol, util
      ├─ pumpkin-inventory ──► data, nbt, protocol, util
      ├─ pumpkin-gametest ──► data, macros, nbt, util, world
      └─ plugin host layer:
           pumpkin-wasm-host ──► core, plugin-runtime, wasm-host-common, wasm-host-v0_1, wasm-host-v0_2
           wasm-host-v0_1 / v0_2 ──► config, core, data, inventory, macros, nbt, protocol, util, world, wasm-host-common
           wasm-host-common ──► auth, core, data, inventory, plugin-runtime, util, world
           pumpkin-plugin-utils ──► plugin-api          (plugin-api has NO internal dependencies)
```

Leaves (no internal dependencies): `pumpkin-api-macros`, `pumpkin-auth`, `pumpkin-codecs`, `pumpkin-macros`, `pumpkin-plugin-api`, `pumpkin-plugin-runtime`, `pumpkin-scheduler`, `pumpkin-fuzzer`.

Leaf edges worth noting:

- `pumpkin-codecs` is depended on by `pumpkin-nbt`, `pumpkin-util`, and through them by everything else.
- `pumpkin-config` is a near-leaf (`pumpkin-util` only) and is depended on by `pumpkin-world` and the wasm hosts.
- `pumpkin-core → pumpkin-gametest` is a real edge in the manifests (the GameTest framework is wired into the server), even though gametest *content* lives in the harness.
- `pumpkin-plugin-api` deliberately depends on nothing internal — it is the permissive-licensed SDK surface, and pulling server code into it would break the license boundary ([INV-0005](../plans/invariants/INV-0005-plugin-crate-license-boundary.md)).
- `crates/pumpkin-plugin-wit` is not a crate: no Cargo.toml, no dependency edges. It is subtree-mirrored to `Pumpkin-MC/pumpkin-plugin-wit` on pushes to `master` (`.github/workflows/sync-wit.yml`).

## External systems

| System | Who talks to it | When |
| --- | --- | --- |
| Mojang piston-meta / piston-data (`piston-meta.mojang.com`) | `crates/pumpkin-data/build.rs` | **At build time.** Downloads the vanilla server jar (version pinned by the repo) to extract the datapack into `assets/datapack/`. A local jar can be supplied via the `PUMPKIN_MINECRAFT_SERVER_JAR` environment variable. |
| Mojang session/registration services | `crates/pumpkin-auth` (reqwest + rustls/ring), plus `PUMPKIN_ACCOUNTS_URL` override in `pumpkin-core`'s pumpkin command | Online-mode player authentication at login. |
| Vanilla Extractor repository (external) | Humans/tools outside this repo | Produces the JSON dumps in `assets/` that `pumpkin-codegen` consumes. Data changes start there. |
| `pumpkin-plugin-wit` mirror (`Pumpkin-MC/pumpkin-plugin-wit`) | CI (`sync-wit.yml`, GitHub App token) | Every push to `master` touching the WIT; external SDK bindings are generated from the mirror. |
| Wasmtime (Wasm runtime) | `pumpkin-wasm-host*` | Loading and running Wasm plugins; includes `wasmtime-wasi` HTTP use. |
| Tokio + Rayon + rustls/ring | whole server | Runtime model: async IO on Tokio, CPU-heavy work on Rayon, TLS via rustls with the ring provider installed explicitly at startup (`main.rs`) and in auth/data clients. |

## Runtime deployment dependencies

- The `Dockerfile` does **not** build from source: it downloads a prebuilt release binary from GitHub Releases into an `ubuntu:26.04` base, exposes **25565/tcp** (Java) and **19132/tcp+udp** (Bedrock), runs as UID/GID 2613 with `RUST_BACKTRACE=1`, and health-checks TCP 25565 via netcat.
- `docker-compose.yml` mounts the data directory at `/pumpkin`, runs read-only rootfs with dropped capabilities and `no-new-privileges`, and keeps stdin open so console commands work.
- Release binaries are produced by CI for Linux x64/ARM64, Windows x64/ARM64, macOS ARM64 and Android aarch64 (`.github/workflows/rust.yml`).

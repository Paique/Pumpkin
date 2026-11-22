# Running the server

> **Status:** Implemented — commands and defaults verified against `crates/pumpkin/src/main.rs`, `crates/pumpkin-config/src/` and `Dockerfile`/`docker-compose.yml` (2026-11-21).

## From source

Run from a **scratch directory** — the server writes `pumpkin.toml`, the world folder and crash reports into the working directory, not the repo:

```sh
mkdir scratch && cd scratch
cargo run --manifest-path /path/to/Pumpkin/Cargo.toml -p pumpkin
```

Wait for the "done" line in the log, send console commands over stdin (`stop` shuts down). Debug builds are fine for iteration; the release profile uses fat LTO and takes much longer to build (and a debug build loudly warns that it must not be used for performance testing).

## First run behaviour

- `pumpkin.toml` is created with defaults; on later runs the file is merged with defaults and rewritten when keys are missing (`pumpkin-config`'s `merge_with_default_toml`).
- Default listeners (all from `pumpkin-config/src/networking/`):
  - Java Edition: `0.0.0.0:25565`
  - Bedrock Edition: `0.0.0.0:19132` (TCP+UDP)
  - Query: `0.0.0.0:25565` (independently toggleable)
- World folder: `basic.default_level_name`, default `world`.

## CLI flags (from `crates/pumpkin/src/main.rs`)

| Flag | Effect |
| --- | --- |
| `--init-settings` / `--initSettings` | Write `pumpkin.toml` and exit with code 0. |
| `--convert-world <format>` | Convert the configured world folder to `pump`, `anvil` or `linear`, print stats, exit. Unknown format → error + exit 1. |

Anything else on argv is ignored (there is no general CLI parser in the server binary itself).

## Environment variables the server reads

| Variable | Effect |
| --- | --- |
| `RUST_LOG` | Overrides the configured log level via `tracing`'s `EnvFilter` (used only if `advanced.logging.enabled`; a full filter directive string is honored via `EnvFilter::try_from_default_env`). |
| `PUMPKIN_ACCOUNTS_URL` | Overrides the accounts service URL used by the `pumpkin` console command's account flow (`pumpkin-core/src/command/commands/pumpkin.rs`). |
| `RUST_BACKTRACE` | Standard backtrace control; the Docker image sets it to `1`. Panic handling always captures backtraces for crash reports. |
| `PUMPKIN_MINECRAFT_SERVER_JAR` | Build-time only: local vanilla server jar for the datapack extraction ([setup.md](setup.md)). |

## Console and signals

- Console input (stdin, optionally rustyline with completion when `advanced.commands.use_tty` and stdin is a terminal) goes through the command dispatcher; `stop` shuts down.
- Ctrl+C (SIGINT): graceful stop; a second interrupt while already stopping force-exits. SIGTERM/Ctrl+Close/Ctrl+Shutdown: graceful stop. (Windows Ctrl+Logoff deliberately ignored.)

## Docker

The repo `Dockerfile` runs a **prebuilt release binary** (downloaded from GitHub Releases at image build, selectable via `PUMPKIN_TAG`/`USE_X64_V3` build args), not a source build:

```sh
mkdir -p data && sudo chown -R 2613:2613 data   # server runs as UID/GID 2613 on Linux
docker compose up
```

Exposed: `25565/tcp`, `19132/tcp`, `19132/udp`. Data at `/pumpkin` (mount volume), read-only rootfs, health check = TCP connect to 25565. `docker-compose.yml` keeps `stdin_open: true`/`tty: true` so console commands work via `docker attach`/`docker compose attach`.

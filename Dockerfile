# syntax=docker/dockerfile:1

# Multi-stage build that compiles Carrot from source. The published image is
# self-contained: every push to main builds this and publishes it to ghcr.
# Pterodactyl/Wings injects the startup command as the container command, so
# this image intentionally declares no ENTRYPOINT or CMD.

# --- Build stage -----------------------------------------------------------
# The official rust image ships the stable toolchain the project targets.
# RUSTUP_TOOLCHAIN skips rust-toolchain.toml so rustup does not download the
# rust-analyzer/rust-src components it lists, which the build does not need.
FROM rust:1-bookworm AS builder

ENV RUSTUP_TOOLCHAIN=stable

WORKDIR /build

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY tools ./tools
COPY assets ./assets

# Cache mounts keep the cargo registry and build artifacts across builds.
# BuildKit cache mounts + GitHub Actions cache make incremental CI builds
# realistic for a workspace this size.
RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=cargo-target,target=/build/target \
    cargo build --release --bin pumpkin && \
    cp target/release/pumpkin /usr/local/bin/pumpkin

# --- Runtime stage ----------------------------------------------------------
FROM ubuntu:26.04

RUN apt-get -o Acquire::Retries=3 update && \
    DEBIAN_FRONTEND=noninteractive apt-get -o Acquire::Retries=3 install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/* && \
    groupadd -g 988 container && \
    useradd -u 988 -g container -m -d /home/container container

COPY --from=builder /usr/local/bin/pumpkin /usr/local/bin/pumpkin

WORKDIR /home/container
USER container:container

ENV RUST_BACKTRACE=1

# Java Edition and Bedrock (UDP).
EXPOSE 25565 19132/udp

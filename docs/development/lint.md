# Linting and static analysis

> **Status:** Implemented — verified against the workspace `[workspace.lints]` in `Cargo.toml`, `.github/workflows/rust.yml` (RUSTFLAGS, clippy jobs), `.github/workflows/typos.yml` and `typos.toml` (2026-11-21).

## Clippy — intentionally strict

The workspace denies entire clippy groups plus individual lints (`Cargo.toml` → `[workspace.lints.clippy]`):

- Groups **denied**: `all`, `nursery`, `pedantic`, `cargo`.
- Individually denied include: `unwrap_used`, `expect_used`, `panic`, `todo`, `print_stdout`, `print_stderr`, `dbg_macro`, `rc_buffer`, `use_self`, `verbose_file_reads`, `if_then_some_else_none`, `redundant_test_prefix`, `equatable_if_let`, `branches_sharing_code`, `useless_let_if_seq`, and more (read the file for the full list).
- Cargo-level: `unused_dependencies` and `unused_workspace_dependencies` are denied.

Local check (match the crates you touched and their dependents):

```sh
cargo clippy -p <crate> --all-targets --all-features
```

CI runs, with `RUSTFLAGS="-Dwarnings"` making any warning an error:

```sh
python .github/scripts/ci_packages.py run -- cargo clippy --all-targets --all-features   # affected packages
cargo clippy --release --all-targets --all-features                                      # full runs (pushes)
```

### Exceptions

- The sanctioned escape hatch is a local `#[expect(..., reason = "...")]` (AGENTS.md) — never a bare `#[allow]`, never removing the reason.
- Test code may allow unwrap/expect/panic via crate-level cfg_attr (pattern used in `crates/pumpkin/src/main.rs`).
- `tools/pumpkin-fuzzer` deliberately relaxes the clippy denies; it is a standalone tool, and its relaxation does not apply to server crates.

## Compiler warnings

CI sets `RUSTFLAGS: "-Dwarnings"` for every job — a warning anywhere fails CI. This is the enforcement behind [INV-0004](../plans/invariants/INV-0004-zero-warnings-ci.md). Pre-existing warnings in files you didn't touch are tolerated locally; adding new ones is not (AGENTS.md "Code quality").

## Typo check

```sh
typos            # crates.io: typos-cli; config in typos.toml
typos -w         # fix in place
```

`.github/workflows/typos.yml` runs `crate-ci/typos` over the whole repository on every push and PR.

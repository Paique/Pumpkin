# INV-0004: CI compiles warning-free

- **Status:** Enforced
- **Scope:** all workspace crates and tools

## Statement

Every CI job compiles with `RUSTFLAGS="-Dwarnings"`: any compiler warning, anywhere in the code that CI builds, fails the build. Pre-existing warnings in files a change doesn't touch are tolerated locally but new warnings are not.

## Rationale

Warnings are deferred bugs and lint noise that hide the signal reviewers need; the project's clippy policy is intentionally strict, and a warning budget would erode it one exception at a time. AGENTS.md states it directly: "Add no new clippy warnings."

## Enforcement

- `.github/workflows/rust.yml` sets `RUSTFLAGS: "-Dwarnings"` globally for all cargo jobs (format, clippy debug/release, tests, builds).
- Clippy runs with `--all-targets --all-features` on affected packages (and workspace-wide on full runs), so lint warnings are also hard errors ([lint.md](../../../docs/development/lint.md)).
- Exceptions exist only as scoped, reasoned `#[expect(...)]` attributes in source, never as CI configuration.

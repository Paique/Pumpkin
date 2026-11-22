# Formatting

> **Status:** Implemented — verified against `rustfmt.toml` and `.github/workflows/rust.yml` (2026-11-21).

```sh
cargo fmt --check    # what CI enforces
cargo fmt            # apply
```

- The only `rustfmt.toml` setting is `edition = "2024"` — formatting is otherwise rustfmt defaults.
- CI's `format` job runs `cargo fmt --check` on the whole workspace and gates the test jobs on it.
- Do not reformat code you didn't touch (AGENTS.md "Scope"): formatting churn in unrelated lines is treated as mixing unrelated changes.
- Spelling/formatting of *text* is handled separately by `typos` — see [lint.md](lint.md).

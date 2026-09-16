# 0002 — CI gates and docs-only skip

- **Status:** Accepted
- **Date:** 2026-06-28
- **Provenance:** commit `e25e9d8`

## Background

CI should gate every code change on the same three checks the project's developers run locally, without wasting runs on commits that cannot affect the build.

## Decision

- `ci.yml` runs on push/PR and gates `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`.
- CI **skips docs-only changes** via `paths-ignore` (markdown, `assets/`, issue templates, `.gitignore`). A commit that also touches code still runs. `web/` is deliberately **not** ignored (it's embedded in the binary).
- A `concurrency` group cancels superseded in-progress runs on the same ref.

## Reasoning

- `clippy -D warnings` keeps the tree lint-clean; a pre-existing lint can silently fail the gate otherwise.
- `web/` is embedded via rust-embed, so it affects the produced binary.
- Cancelling superseded runs saves CI minutes on fast iteration.

## Alternatives considered

- Run CI on every commit including docs-only — rejected: wasted minutes, no signal.
- No concurrency cancellation — rejected: redundant runs on stacked pushes.

## Consequences

- Contributors keep all three checks green locally; CI mirrors that.
- Docs-only PRs skip CI entirely.
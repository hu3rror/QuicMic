# 0001 — Release pipeline with cargo-dist & crates.io publishing

- **Status:** Accepted
- **Date:** 2026-06-28 (cargo-dist migration) → 2026-07-12 (crates.io publishing)
- **Provenance:** commits `45928a9` (migrate to cargo-dist), `eedb9dd` (tag push trigger), `00e1a15` (crates.io Trusted Publishing)

## Background

Multi-platform release builds were needed for x86_64 + arm64 on Linux, Windows, and macOS, with third-party license attribution ($THIRD_PARTY_LICENSES) attached to every release. The crate is also published on crates.io, but dist ships binaries, not source packages.

## Decision

- Manage releases with **cargo-dist** (`dist`). `.github/workflows/release.yml` is **generated** — never hand-edit it; change `dist-workspace.toml` and run `dist generate`.
- `installers = []` (plain archives, no install scripts).
- All six targets built **natively** (cargo-dist picks current runners; `[dist.github-custom-runners]` pins arm64 Windows to `windows-11-arm` rather than xwin cross-compiling).
- `[dist.dependencies.apt] libasound2-dev` provides cpal's ALSA headers on Linux.
- Releases trigger on **pushing a `vX.Y.Z` git tag** (plus a `pull_request` plan-only check; no `workflow_dispatch`).
- Third-party license file wired as a `[[dist.extra-artifacts]]` running `cargo about generate`.
- `[profile.dist]` (`lto = "thin"`) is the dist release profile.
- crates.io publishing runs in a **custom publish job** (`.github/workflows/publish-crates.yml`, wired via `publish-jobs = ["./publish-crates"]`) at the publish stage with `needs: [plan, host]` — i.e. **after** binaries are built and the GitHub Release exists. Needs a `CARGO_REGISTRY_TOKEN` repo secret; dist passes `secrets: inherit` automatically. Adding/changing a publish job means re-running `dist generate`.

## Reasoning

- A failed build must never burn an immutable crates.io version (a published version can only be yanked, never replaced) — hence publish-after-build.
- Native builds avoid dead/cross-compiled runners.
- Generated workflow keeps the config in one editable place (`dist-workspace.toml`).

## Alternatives considered

- Hand-maintained `release.yml` per OS — rejected: duplicated matrix logic, drift, no maintainable single source.
- Cross-compile arm64 Windows from Linux via xwin — rejected: dead-runner risk, complexity.
- Manual crate publishing — rejected: version burn risk on failure.

## Consequences

- Releasing: bump `version` in `Cargo.toml`, commit/push, then push the matching tag (`git tag v0.2.0 && git push origin v0.2.0`) — the tag, not the Cargo.toml commit, fires the release.
- Uses the built-in `GITHUB_TOKEN` (no PAT).
- `ci.yml` still gates fmt/clippy/test on push/PR (see ADR-0002).
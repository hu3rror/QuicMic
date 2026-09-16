# Developer & Agent Guide: QuicMic

QuicMic turns a web browser into a low-latency wireless PC microphone over LAN: an HTML5/AudioWorklet client captures mono audio and streams it over WebTransport (primary) or WebSocket (fallback) to a Rust server (Axum + wtransport) that plays it into a virtual audio device.

For the full picture, read in this order: `CONTEXT.md` (domain terms) → `docs/architecture.md` (how the system works today) → `docs/adr/` (why, one file per decision). This file holds only the rules that must be obeyed when changing the code.

## Rules (hard guardrails)

1. **Source-file comments & docs in English** (`.rs`, `.js`, `.toml`).
2. **Never hold a `std::sync` or `parking_lot` MutexGuard across an `.await`** — it makes the Future `!Send` and fails Axum's `Handler` bounds. Drop the guard or scope the lock.
3. **The SPSC ring buffer allows exactly one producer and one consumer.** Adding a second is a data race; the `unsafe impl Send + Sync` is only valid under the SPSC contract.
4. **Share hot floats as `Arc<AtomicU32>`** with `f32::to_bits()` / `f32::from_bits()` (std has no `AtomicF32`).
5. **All client HTTP fetches go through `fetchWithTimeout`** (default 1000ms) so a LAN-unreachable server never leaves a request pending.
6. **Keep the crypto tree pinned to `ring`**: never re-add default-featured `rustls` / `axum-server` / `tokio-rustls` — it silently drags in `aws-lc-sys` (needs a C compiler + NASM), breaking the w64devkit gnu build. (ADR-0014)
7. **`release.yml` is generated**: change `dist-workspace.toml` and run `dist generate`; never hand-edit the workflow. (ADR-0001)
8. **Feature-detect the WebTransport datagram writer**: `createWritable()` first, legacy `writable` fallback — Safari/iOS 26.4+ throws `TypeError` on the legacy property. (ADR-0005)
9. **Audio health = FLOW ∧ FLAGS.** `isAudioHealthy()` must require *both* the worklet actually posting (`AUDIO_STALL_MS`) and the track/context flags — never one alone; any new health signal is a *term*, never a replacement for flow. The muted path must keep heartbeating (`MUTED_HEARTBEAT_EVERY`, bare `{level:0}`, no DSP). One decision point (`checkAudioHealth()` on the existing 1s tick); events are diagnostic. Never grab the mic while the page is hidden, never loop recovery, never watch packet flow (the gate sends nothing in silence). A failed single recovery does a clean `stopForAudioLoss()` rather than hanging in "connected but silent". (ADR-0003)
10. **Preserve the single-connection contract**: every audio session must acquire the slot via the CAS guard and release it on every exit path; F5 handover depends on `/api/renew` + the broadcast cancel. (ADR-0007)
11. **Detect disconnects over HTTP, never from transport close events** (they arrive seconds late on iOS Safari): `503`/unreachable = server gone; `401` from `/api/stats` = session taken over (re-pair in place); a confirmed server-gone locks pairing behind a Reload button. Liveness cadences: 1s streaming poll, ~3s idle and Eco Mode checks. (ADR-0009)
12. **Keep the wire stream contiguous**: the client-side gate/gain owns all per-sample DSP and the sequence number is only incremented for *sent* packets; the server is a pure passthrough. (ADR-0008)
13. **When touching `write_data`/`ResamplerState`**: the resampler unit tests are an exact-behaviour safety net — keep them green. Keep `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` green; CI gates on all three. (ADR-0002)

## Docs

- `CONTEXT.md` — domain model & glossary (read before exploring; ADR conflicts must be flagged, not silently overridden).
- `docs/architecture.md` — how the system works today: audio pipeline, packet format, client mechanisms, statistics, auth surfaces, module map, key constants.
- `docs/adr/` — one file per design decision (TLS pinning, single-connection, client-side DSP, HTTP disconnect detection, audio health model, output supervisor, static serving, per-IP throttle, update check, ring crypto, release pipeline, CI, fake-ip filtering, createWritable). Read the ones touching the area you're about to modify.

---

## 🌿 Fork Development Workflow (hu3rror/QuicMic)

Fork-local branch conventions. **This section is fork-specific guidance** — keep it out of PR branches (PR branches are based on `main`, which never contains it).

- **`main`** — always identical to `upstream/main`. Sync only via `git fetch upstream && git merge --ff-only upstream/main`; never commit directly to `main`.
- **`dev`** — the living development branch (tracks `origin/dev`). All new feature/fix work is committed here first. Regularly merge `main` into it so it stays current with upstream.
- **`pr/<name>`** — one branch per upstream PR, **always based on `main`** (== upstream) and carrying _only_ the PR-ready commits, so the PR diff contains exactly that change and never dev's other WIP:
  ```bash
  git checkout -b pr/<name> main
  git cherry-pick <commits>…        # the PR-ready commits from dev
  git push origin pr/<name>
  gh pr create --repo Fix3dll/QuicMic --base main --head hu3rror:pr/<name>
  ```
  If `dev` holds exactly one change beyond `main`, `git rebase --onto main dev` on a scratch branch is equivalent — but the PR branch must still start from `main`, not `dev`.
- **`release/vX.Y.Z`** — personal releases on the fork, never PR'd (branch from `dev` or `main`, merge what you want into it; tag with `git -c tag.gpgSign=false tag -a vX.Y.Z -m ...`).

Sync routine (run at the start of each working session):
```bash
git checkout main && git fetch upstream && git merge --ff-only upstream/main && git push origin main
git checkout dev && git merge main && git push origin dev
```

After an upstream PR merges (maintainer may rebase/rewrite the commits), sync `main` then merge into `dev`; `dev` may temporarily hold now-duplicate content — reconcile by dropping the commits already absorbed upstream.

## Agent skills

### Issue tracker

Issues and specs for this repo live as GitHub issues (tracked with the `gh` CLI). See `docs/agents/issue-tracker.md`.

### Triage labels

Five canonical triage roles mapped to labels: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout: one `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
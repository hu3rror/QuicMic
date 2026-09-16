# 0003 — Client-side audio health model: flow ∧ flags

- **Status:** Accepted
- **Date:** 2026-07-12
- **Provenance:** commit `c550c13` (recover from microphone interruptions)

## Background

Three separate shipped bugs shared one root cause: the reconnect path only rebuilt the *transport*, and nothing ever checked whether the *microphone* was still alive. The client toasted "Reconnected!" over dead silence. A further bug showed an OS-muted track still pumps (silence), and iOS can leave a "zombie graph" where every flag reads healthy yet nothing is ever pumped.

## Decision

- **Health = FLOW ∧ FLAGS.** `isAudioHealthy()` requires both:
  - **flow** — `isAudioFlowing()`: did the worklet post within `AUDIO_STALL_MS` (4s)?
  - **flags** — `track.readyState === 'live'`, `!track.muted`, `audioContext.state === 'running'`.
- Any new health signal is added as a *term*, never as a replacement for flow.
- **The worklet heartbeats even while muted** (`MUTED_HEARTBEAT_EVERY` ~1/s, bare `{ level: 0 }`, skipping DSP). The muted path must never be fully silent.
- **One decision point: `checkAudioHealth()`**, on the 1s tick that already exists — no new timer, and it runs while muted too. Events (`onended`, `onmute`, `onstatechange`) are **diagnostic**; they do not each act (that produced duplicate WARN spam).
- **An OS-muted track means WAIT, not recover** (a call or another app owns the mic). One exception: `visibilitychange → visible` does reclaim.
- **Never watch packets** — the noise gate sends nothing during silence, so packet flow is not a liveness signal.
- **Recovery is an escalating ladder**, each rung verified by `waitForAudioFrame()` (the worklet actually posting again): `resume()` → re-acquire the mic → **full graph rebuild** (`teardownAudioGraph()` + `setupAudioGraph()`). One pass, foreground only.
- **Never grab the mic in the background.** `recoverAudio()` returns immediately unless the page is visible, and makes exactly one attempt — never a loop.
- **Fail honestly, never hang.** If the single recovery attempt fails, `stopForAudioLoss()` performs a clean full stop with a clear message.
- **"Transport connected" ≠ "microphone live".** `reconnectLoop` verifies `isAudioLive()` (and tries one recovery) before it may toast "Reconnected!".
- **One WARN per interruption episode** (`interruptionNotified`), cleared only once audio is flowing again.
- `POST /api/client-state` reports *why* audio went quiet: `track.onended` → `mic_lost`, `track.onmute` → `mic_interrupted`. Sent with `navigator.sendBeacon` when hidden (browser flushes it even as JS freezes), otherwise a normal `fetch`.
- Backgrounding the page does **not** stop capture; the OS taking the mic does. Being hidden is NOT a microphone event (`visibilitychange → hidden` produces no alarm).

## Reasoning

Each term catches exactly what the other misses: flow alone is blind to an OS-muted track (silence still pumps); flags alone are blind to a zombie graph (all flags healthy, nothing pumped). Packet-watchdogs false-positive on every silence.

## Alternatives considered (rejected)

- Trust only `track.readyState`/`muted`/`audioContext.state` — rejected: the zombie-graph bug shipped because all flags read healthy while audio was dead.
- Trust only flow — rejected: an OS-muted track still pumps (silence), so flow alone reports healthy over a muted mic.
- Let each event handler act independently — rejected: duplicate WARN spam for one interruption.
- Watchdog on packet flow — rejected: false-positives on every gated silence.
- Recover by retrying harder / looping — rejected: fights the OS/app that owns the mic.

## Consequences

- The client never leaves the user in "connected but silent" limbo.
- One interruption = one server WARN with a cause, e.g. `WARN Client lost microphone access (taken by another app) — no audio until the user resumes`.
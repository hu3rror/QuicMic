# 0008 — Client-side noise gate & gain; server is a pure passthrough

- **Status:** Accepted
- **Date:** 2026-06-27
- **Provenance:** commit `643d4fe` (initial commit)

## Background

Audio quality settings (noise gate, gain) need to exist for the product, and silence should not be shipped over the LAN (battery/heat on the phone's Wi-Fi radio). The cleanest place for per-sample DSP is the audio thread that already owns the signal.

## Decision

- The noise gate **and gain** run **client-side** in the AudioWorklet (`web/worklet.js`); the server applies **no per-sample DSP** — it just decodes little-endian i16 samples into the ring (`audio::decode_into_ring`).
- Gate: per-sample² threshold, 250ms hold; only packets that pass are `postMessage`d, so during silence the client sends **nothing** (the Wi-Fi radio idles) and the main thread isn't woken. A throttled `{ level }` message still drives the VU meter to zero.
- **The sequence number is only incremented for sent packets**, so the server's `LossTracker` sees a contiguous stream and never reports false loss.
- **Onset look-ahead**: the last gated packet is prepended on a closed→open edge so a word's soft attack isn't clipped (the prepended packet takes the next sequence number, keeping the stream contiguous).
- Threshold pushed to the worklet via a `{ type: 'gate', threshold }` port message on stream start and whenever the dB slider changes (`sendGateToWorklet`). Mute is signalled with `{ type: 'mute' }` so the worklet skips all processing while muted.
- Gain applied in the float domain before Int16 conversion (no double quantization).
- Settings are still stored server-side (`noise_gate`, `gain` as `Arc<AtomicU32>` using `f32::to_bits()`/`from_bits()`, `latency_threshold` as ms) — for persistence and cross-device sync. The client (`localStorage`) is the **source of truth**: it restores saved settings on page load and pushes them to the server on pairing/token renewal/reconnection; `loadSettings` only adopts server values on a true first run, so a freshly restarted server (CLI defaults) never clobbers the user's choices.
- The worklet emits the **full wire frame** — a 4-byte header gap (`HEADER_BYTES`, 2-byte aligned for the PCM `Int16Array` view) followed by Int16 PCM — and transfers it; the main thread only stamps the sequence number into the gap and sends the buffer as-is (no per-packet allocation/copy on the main thread).
- The structured port protocol (`gate` / `gain` / `mute` / `frame+level` / `level`) is the extension point for future audio-thread features.

## Reasoning

Deciding on the raw signal and gating before encoding means the server never receives gated-out silence and the phone's radio idles — a real battery/heat win, and it removes the server hot path entirely. Keeping sequence numbers contiguous (only sent packets counted) preserves loss statistics.

## Alternatives considered

- Server-side gate/gain — rejected: server would receive and process silence (radio stays hot, CPU spent on gated samples).
- Increment sequence numbers for gated-out packets too — rejected: false loss reports on the server.

## Consequences

- "No packets" during silence is correct behavior, never evidence of a fault (see ADR-0003).
- CLI: `--noise-gate` is given in dB (-100 = Off..0), clamped to [-100, 0] and converted to linear amplitude; `--gain` / `--latency-threshold` clamped to the shared `server::*` bounds.
# 0010 — Output-device supervisor owns the `!Send` cpal stream

- **Status:** Accepted
- **Date:** 2026-06-27
- **Provenance:** commit `643d4fe` (initial commit)

## Background

The cpal audio output stream is `!Send`, and the target virtual device can disappear mid-stream (e.g. a disabled virtual cable — on Windows this surfaces as WASAPI `AUDCLNT_E_DEVICE_INVALIDATED`). Playback must resume with no restart, and a bad `--device` name at startup must stay a fatal error.

## Decision

- **`spawn_output_supervisor`**: a dedicated thread owns the stream's whole lifecycle.
- If the device fails mid-stream, the stream's error callback signals the supervisor over an `mpsc` channel; the supervisor drops the dead stream and rebuilds it, retrying once per second until the device is back — playback resumes **with no restart**.
- While the device is down, `StreamState.device_ok` is cleared and reported via `/api/stats` (`audio_device_ok`) so the web UI can warn.
- The initial build result is reported back to `main`, so a bad `--device` name stays a fatal startup error exactly as before.

## Reasoning

The `!Send` constraint forces the stream onto one thread; the mpsc error path turns a device loss into a supervised rebuild instead of a dead stream or a crash.

## Alternatives considered

- Recreate the stream inline in the audio thread — rejected: no error channel, no retry, blocks on the device.
- Let the stream die and require a restart — rejected: playback must resume without user action.

## Consequences

- `audio_device_ok: false` is a live, polled signal for the UI while the supervisor rebuilds.
- Startup validation semantics are preserved.
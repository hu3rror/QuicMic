# QuicMic — Architecture (how the system works today)

This is the mechanism reference: what the system does and where it lives. For the *why* behind these mechanisms, read the ADRs in `docs/adr/` that touch the area you're modifying. For the domain vocabulary, see `CONTEXT.md`.

## Data flow at a glance

```
[Microphone] → AudioWorklet (gate + gain + Int16) → [seq stamp] → WebTransport (primary) / WebSocket (fallback)
                                                                        ↓ LAN
[decode_into_ring] → SPSC ring buffer → cpal output thread → Catmull-Rom resampler → channel duplication → virtual device
```

## Packet format

- 4-byte little-endian **sequence number**, then raw **i16 PCM** samples (mono), up to 480 samples (10ms @ 48kHz).
- The worklet emits the full wire frame (4-byte aligned header gap + PCM) and transfers it; the main thread only stamps the sequence number and sends — no per-packet allocation or copy (ADR-0008).
- Sequence numbers increment **only for sent packets** — gated-out silence never consumes one (ADR-0008).

## Server-side audio (`src/audio/`)

- `ring_buffer.rs` — lock-free SPSC ring over `UnsafeCell<i16>`, `Acquire`/`Release` on head/tail, batch `ptr::copy_nonoverlapping`. Exactly one producer + one consumer, ever (SPSC contract).
- `processor.rs` — `decode_into_ring`: parse little-endian i16 and push; a pure passthrough, no per-sample DSP (ADR-0008).
- `output.rs` — cpal output thread:
  - **Catmull-Rom cubic fractional resampling** to the device's native rate (2-sample group delay from the one-sample look-ahead window).
  - **Dynamic source sample rate** via `Arc<AtomicU32>` — the client reports its capture rate (`sr` parameter) and the ratio adjusts in real time.
  - **Latency recovery (hard skip)**: when ring depth exceeds `latency_threshold` (ms → samples at the active source rate), discard the oldest samples down to the prebuffer low-water mark (~30ms real time at any 44.1k–192k rate). 3-second cooldown prevents stuttering.
  - **Channel duplication**: mono → all device channels.
  - **Chunked drain**: source samples drained in `RESAMPLE_CHUNK` (64) batches carried in `ResamplerState`, amortizing per-sample atomic sync (a win on weak-memory CPUs; behaviour-neutral on x86). Leftover batch samples persist across callbacks.
  - Owned by **`spawn_output_supervisor`** — dedicated thread, mpsc error channel, rebuild-with-retry on device loss (ADR-0010).

## Client mechanisms (`web/`)

- **Capture & worklet**: mono capture at the browser's native rate; the worklet gates, gains (float domain, no double quantization), and posts only passing frames plus throttled `{ level }` messages. Worklet port protocol: `gate` / `gain` / `mute` / `frame+level` / `level`.
- **Eco Mode**: solid black overlay (OLED off) + `navigator.wakeLock('screen')` (re-acquired on `visibilitychange`), throttled VU DOM updates (≤ once/100ms), full stats-UI suspension behind the overlay with a ~3s liveness check.
- **Mute**: long-press 500ms (50ms vibration haptic); releasing after a long-press must not toggle the stream off. When muted the worklet keeps running and heartbeating (`MUTED_HEARTBEAT_EVERY` ~1/s, bare `{ level: 0 }`, no DSP); the ring underruns → silence.
- **Settings UI**: ⚙️ panel — noise gate slider (-100 dB [Off]..0, default -50), gain (0.2x–3.0x, default 1.0), latency recovery (0–500ms, default 150), per-value reset buttons.
- **Client storage**: `localStorage` is the source of truth for settings; pushed to the server on pairing / token renewal / reconnection; server values adopted only on a true first run (ADR-0008). The session token is mirrored there too, through a fail-open wrapper (ADR-0017): quota/security failures (private mode, blocked cookies) degrade to a session-only in-memory copy instead of breaking pairing.
- **Entry routing & resume** (ADR-0017): a URL hash (QR) always means explicit pairing intent — the stored token is discarded and the scanned PIN is used. Without a hash, a stored token is **validated** against `/api/stats` (never rotated) before the main screen appears: `200` resumes, `401` re-pairs in place, `503`/unreachable enters a bounded wait (~3s retry × ~60s cap) that resumes automatically when the server returns and falls back to the Reload lock on timeout. A page load is deliberately not a session handover — token rotation happens only at pairing, stream start, and reconnect. The pure routing/verdict/wait/storage decision logic lives in `web/session.js` (unit-tested with Node's built-in runner); `app.js` keeps the fetch/DOM/timer glue.
- **Datagram writer (WebTransport)**: acquired via feature-detect — `createWritable()` first, legacy `writable` fallback, never either API without a capability check (Safari/iOS 26.4+ throws `TypeError` on the legacy property) — see ADR-0005.

## Pairing & network

- Startup prints a QR with `https://<lan-ip>:8443#<pin>` — the PIN lives in the URL **hash**, never sent in HTTP requests. `qr2term` renders it.
- IPv6: server URLs bracket literals per RFC 3986 (`https://[fe80::1]:8443#…`); WebTransport binds dual-stack (`[::]`); cert registers the IP literal as an IP SAN. `--ip` accepts bare or bracketed form. Link-local `%zone` addresses are out of scope.
- LAN IP detection: default-route pick gated by usability checks, else full interface scan with ranking — see ADR-0004 for the decision; `--ip` is the manual override.
- TLS: self-signed ECDSA P-256 cert, **persisted across restarts** (reused when the LAN IP is unchanged). The client pins the SHA-256 of the DER via `serverCertificateHashes` fetched from `/api/info`; 14-day validity ceiling; `--dump-certs` for debugging (ADR-0006). Persisted identity lives in the platform data directory (`%LOCALAPPDATA%\QuicMic` Windows, `~/Library/Application Support/QuicMic` macOS, `$XDG_DATA_HOME/QuicMic` or `~/.local/share/QuicMic` Linux; override with `--data-dir` / `QUICMIC_DATA_DIR`) as `identity.json` + a separate `pin` file. On startup the server loads or generates it, regenerates the certificate when the LAN IP changed or its age reached 13 days, and reuses the stored PIN (`--pin` writes through; `--pin random` regenerates). Rotation happens at startup only — a process up past ~14 days serves an expired cert until restart (ADR-0015).

## Statistics & monitoring

- `GET /api/stats` — **requires the session token** in the `X-Session-Token` header. Returns:
  ```json
  {
    "packets_received": 12345,
    "packets_lost": 2,
    "loss_percent": 0.016,
    "buffer_level": 480,
    "buffer_ms": 10,
    "buffer_capacity": 24000,
    "connected": true,
    "audio_device_ok": true
  }
  ```
  - `buffer_ms` = `buffer_level * 1000 / source_sample_rate` (accurate for non-48kHz sources). `buffer_capacity` from `ring.capacity()`.
  - `audio_device_ok` is `false` while the output device is lost and the supervisor rebuilds (ADR-0010).
  - The shutdown `503` takes precedence over the token gate (ADR-0009).
- **Loss tracking (WebTransport only)**: `LossTracker` — a 64-packet (`REORDER_WINDOW`) received-bitmap; a seq counts as lost only when evicted from the window unseen (reordering absorbed). Duplicates ignored; u32 wrap handled via wrapping arithmetic; forward jump > `MAX_FORWARD_GAP` re-baselines. Confirmed losses surface via `/api/stats` and a rate-limited `warn!` (≤ 1/s). WebSocket runs over reliable TCP → counts received only, never loss.
- **Per-session counters**: `packets_received` / `packets_lost` reset when a new connection is acquired (both transports) — stats reflect the current stream.
- Client polls `/api/stats` every second (this doubles as the primary shutdown detector, ADR-0009); RTT is the poll request's round-trip time over the reused keep-alive connection.

## Auth surfaces (token locations)

- `/api/pair` — 6-digit PIN (per-IP throttled, ADR-0012).
- Session token in `X-Session-Token` **header**: `/api/stats`.
- Token in **body**: `/api/settings` POST, `/api/client-state` (*must* be body — `navigator.sendBeacon` cannot set headers).
- Token in **query**: `/ws`.
- WebTransport session path: invalid/missing token → `session_request.forbidden()` (403); slot busy → `too_many_requests()` (429); teardown immediately. Intentionally **no per-IP accept rate-limiting** on the WebTransport path (LAN trust model + QUIC address validation + single-connection CAS deemed sufficient).
- **Persistence (ADR-0016)**: the session token lives in the data directory as a `session-token` artifact; pairing and `/api/renew` write it through atomically, and startup seeds the in-memory slot from it — so a plain restart keeps the loaded page paired. A PIN-reset boot (`--pin random` / deleted `pin` file) clears it.

## Module map

- `src/main.rs` — thin wrapper over `run()`: prints the full error chain on `Err`; on Windows, if launched by double-click (`GetConsoleProcessList`), waits for Enter so the error is readable. Startup failures from the servers propagate out of the `tokio::select!` as fatal. CLI inputs validated/clamped early (`--pin` exactly 6 digits or `random`, validated in `persistence`; `--data-dir` / `QUICMIC_DATA_DIR` select the data directory; `--noise-gate` dB → linear; `--gain` / `--latency-threshold` clamped to `server::*` bounds).
- `src/persistence.rs` — persisted identity (ADR-0015): data-directory resolution (`--data-dir` > `QUICMIC_DATA_DIR` > platform default) and `IdentityStore`, which loads or generates `identity.json` (cert keypair + `lan_ip` + `created_at`) and the separate `pin` file. Startup rotation when the stored `lan_ip` differs or the cert is ≥ 13 days old (cert regenerated, PIN never touched); corrupt files are quarantined to `<name>.corrupt-<ts>` and regenerated independently; atomic temp-file-plus-rename writes, `0600` on Unix. `--pin <digits>` writes through, `--pin random` regenerates.
- `src/tls.rs` — cert keypair operations: in-memory self-signed generation (ADR-0006), `from_pem` reconstruction with SPKI-matched key validation, and `--dump-certs` export; the load-or-generate orchestration lives in `persistence` (ADR-0015).
- `src/update_check.rs` — zero-dependency release check (ADR-0013). `update_available` / `latest_version` / `releases_url` on `/api/info`; dismissal remembered per version in `localStorage`.
- `src/server/` — `state.rs` (`StreamState`, `AppState`, single-connection lifecycle), `api.rs`, `websocket.rs` (rides the HTTP server), `webtransport.rs` (a **separate** QUIC server, not routed through Axum), `assets.rs` (hybrid serving, ADR-0011), `mod.rs` (router, middleware, HTTPS server). Public re-exports: `StreamState`, `AppState`, `build_router`, `run_https_server`, `run_webtransport_server`; everything else is `pub(super)`/private.

## Key constants

| Constant | Value | Meaning |
| --- | --- | --- |
| `AUDIO_STALL_MS` | 4s | Worklet-post staleness bound for audio health flow |
| `MUTED_HEARTBEAT_EVERY` | ~1/s | Bare `{level:0}` post while muted |
| `PREBUFFER_MS` | ~30ms | Output low-water mark; also the hard-skip trim target |
| `REORDER_WINDOW` | 64 | LossTracker received-bitmap size |
| `MAX_FORWARD_GAP` | (see `processor.rs`) | LossTracker re-baseline threshold |
| `RESAMPLE_CHUNK` | 64 | Ring drain batch per callback |
| `SHUTDOWN_GRACE_PERIOD` | 1200ms | API kept up after shutdown so the client's poll lands on `503` |
| `FAILURE_DECAY` | 60s | Pairing-throttle idle pruning window |
| `HEADER_BYTES` | 4 | Wire-frame seq header (2-byte aligned for the PCM view) |
| `MAX_SAMPLE_RATE` | (see `src/audio/mod.rs`) | Upper bound for source-rate handling |
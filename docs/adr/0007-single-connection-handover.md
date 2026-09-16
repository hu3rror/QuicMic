# 0007 — Single-connection model and F5 refresh handover

- **Status:** Accepted
- **Date:** 2026-06-27
- **Provenance:** commit `643d4fe` (initial commit)

## Background

Only one active audio stream may exist (one phone = one mic), so resource exhaustion and conflicting streams must be prevented. But a browser refresh (F5) makes the client immediately try a *new* connection while the server's old session loop is blocked in `receive_datagram`/`socket.recv()` — detectable only after a timeout that can take seconds. Without a fast clear, the new connection is rejected with "another client is already connected".

## Decision

- **Single-connection limit** via `AtomicBool` CAS: new connections use `compare_exchange(false, true, SeqCst, SeqCst)` with retries (10 × 20ms) to atomically acquire the slot, eliminating TOCTOU races.
- **RAII `ConnectionGuard`**: a `Drop`-implementing struct resets `is_connected = false` when the session task terminates (completion, cancellation, or panic).
- **Token renewal (`/api/renew`)**: before starting a new stream the client hits `/api/renew`; the server generates a new token, stores it in `session_token`, broadcasts a cancellation signal over `cancel_tx`, and sleeps 50ms.
- **Broadcast cancellation** (`tokio::select!`): session loops listen on `cancel_rx.recv()`; a broadcast instantly breaks the old loop and drops its connection, at **0% CPU** during active streaming (no high-frequency timer-wheel allocations as in the earlier 50ms polling loop):
  ```rust
  tokio::select! {
      _ = cancel_rx.recv() => { /* terminate session immediately */ }
      res = receive_datagram() => { ... }
  }
  ```
- **Graceful shutdown (Ctrl+C) is deliberately minimal**: clients detect shutdown by polling the HTTP API — *not* by listening for a transport close frame (unreliable and seconds late on iOS Safari; see ADR-0009). The server does not send `1001` close frames or await any close handshake. `main` simply: sets `is_shutdown` (an Axum middleware `reject_during_shutdown` then replies `503` to every new HTTP request), broadcasts over `cancel_tx` (the session loop breaks and drops its connection — the QUIC/TCP socket closes on its own), keeps the API up for `SHUTDOWN_GRACE_PERIOD` (1200ms) so the client's ~1s liveness poll reliably lands on a `503`, then drains via `axum_handle.graceful_shutdown` and exits.

## Reasoning

The earlier 50ms polling loop allocated timer-wheel entries on the hot path; an event-driven broadcast channel is both faster and 0% CPU while idle. A close-frame handshake was removed once on-device testing showed iOS ignores the close anyway.

## Alternatives considered (rejected)

- 50ms polling loop for stale-session detection — rejected: timer-wheel allocations on the hot path.
- Explicit `1001` close frames + `shutdown_notify` handshake — rejected: iOS Safari ignores the close frame and delivers the event seconds late; detection ends up entirely over HTTP regardless.

## Consequences

- F5 refresh hands over without the "another client is already connected" rejection.
- Connection slot can never leak: any session exit path (normal, cancel, panic) releases it via RAII.
- A shutting-down server answers `503` from the middleware, which also takes precedence over the `/api/stats` token gate (see ADR-0009).
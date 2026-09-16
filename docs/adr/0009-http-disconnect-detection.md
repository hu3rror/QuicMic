# 0009 — Disconnect detection over HTTP, never over the transport close event

- **Status:** Accepted
- **Date:** 2026-06-27 (initial) — extended by ADR-0007 (graceful shutdown) and ADR-0003 (audio health)
- **Provenance:** commit `643d4fe` (initial commit)

## Background

The client must detect a vanished server fast and reliably. On iOS Safari (the primary mobile target), `transport.closed` surfaces **seconds late** and carries no useful code — it resolved as `{clean: true, code: 0}` even when the server sent an explicit `1001`. Datagram `write()` errors are similarly delayed. Neither the close event nor its code is a reliable, timely signal there.

## Decision

Disconnect handling is **transport-agnostic and does not inspect the close code**. The reliable, fast signal is the **HTTP layer**: the client polls `/api/stats` every second while streaming, and the server replies `503` while shutting down. A `503` (or an unreachable server) is treated as a definitive "server gone".

- **Unified close handler**: both `ws.onclose` and `transport.closed` (resolve and reject paths, any code) funnel into a single handler. It ignores stale events (from a replaced transport) or self-initiated ones (user stopped, or a reconnect handover in progress), then runs one fast liveness probe (`/api/info`).
- **Probe decides intent**: probe fails / `503` → server gone → return to pairing screen. Probe succeeds → transient drop → reconnect. A `401` from `/api/stats` means the server is alive but the session was taken over → re-pair **in place** (no reload).
- **Server-gone locks the pairing screen (reload required)**: the self-signed certificate is regenerated every server start (ADR-0006), so the loaded page's pinned hash and accepted cert are stale; in-page re-pairing would silently fail on the cert mismatch and the stale page cannot even probe for the server's return. On a confirmed "server gone" the client locks the PIN input and shows a Reload button (`location.reload()`). Each verdict is confirmed first (a `503` is definitive; a network error or a single idle-check failure triggers one confirming `/api/info` probe), so a transient blip doesn't force a reload.
- **Stats poll is the fast path on iOS**: a streaming `/api/stats` poll returning `503` immediately returns to pairing — shutdown detected within ~1s without waiting on the late transport event. In Eco Mode, a lighter ~3s liveness check plays the same role.
- **Reconnect (transient drops)**: first attempt immediate (0s), then exponential backoff (1s → 2s → 3s → 4s, max 5 attempts). Each attempt renews the session token via `/api/renew` first; the loop bails early once a probe confirms the server is gone.
- The AudioContext and Worklet stay alive during reconnection to minimize resume latency.
- All paths are idempotent: the `isStreaming` / `isReconnecting` / `isConnecting` flags ensure a shutdown observed by several detectors at once still results in a single, clean transition.

## Reasoning

Verified on-device: iOS ignores the close frame and delivers the event late regardless of what the server sends, so any design keyed on the transport close event is unreliable there. The client already polls `/api/stats`, so the HTTP layer costs nothing extra.

## Alternatives considered (rejected)

- Branch on close codes `1000`/`1001`/`0` and string-match error messages — rejected: iOS resolves as `{clean: true, code: 0}` either way.
- Send `1001` close frames and wait on a `shutdown_notify` handshake — rejected: iOS ignores the close; detection ends up entirely over HTTP regardless.
- Re-pair in-page after a server restart — rejected: the stale cert hash fails every HTTP request, so a full reload is the only reliable way back.

## Consequences

- `/api/stats` requires the session token in the `X-Session-Token` header (only the already-paired client polls it), but the shutdown `503` takes precedence — `reject_during_shutdown` middleware runs before the handler, so a shutting-down server answers `503` regardless of token; a `401` instead means the session was taken over while the server is still alive.
# 0017 — Client-side session persistence: validate-only resume, QR-clear pairing, bounded wait

- **Status:** Accepted
- **Date:** 2026-09-16
- **Provenance:** grilling session → spec issue `hu3rror/QuicMic#3`; implemented in the client-session-persistence feature commit

## Background

ADR-0015/0016 made the server-side pairing credential survive restarts (TLS identity, PIN, session token), so a plain restart keeps the loaded page paired — but the client undermined that promise in three ways.

First, the client renewed the token on every page load (`renewSessionToken`). Renewal **rotates** the token (ADR-0007), so opening or reloading a *second* tab of the same origin invalidated the streaming tab's token, silently kicking it to the pairing screen on its next poll: tab-open was a session handover.

Second, `localStorage` writes were unguarded. In a quota-limited context (Android Chrome incognito) `setItem` throws, and the error handling misrouted that exception to "Server closed" — locking the pairing screen behind a Reload button that throws again on every reload: a dead-end loop.

Third, every confirmed server-gone verdict immediately locked the pairing screen behind a Reload button (ADR-0009's default). ADR-0015 made the certificate stable across plain restarts, so that lock is now wrong for the common case — a reboot should be invisible, not a manual Reload.

## Decision

Client-side session persistence with three mechanisms, all living in `web/`:

1. **Validate-only resume (no rotation on load).** Entry routing: a URL hash (QR) always means explicit pairing intent — the stored token is discarded and the scanned PIN is used. Without a hash, a stored token is **validated** against `/api/stats` (never rotated) before entering the main screen; a `200` resumes, a `401` falls to the pairing screen (re-pair in place, ADR-0009), a `503`/unreachable enters the bounded wait. Token rotation now happens only at pairing, stream start, and reconnect. This makes page loads non-handovers: opening a second tab is a non-event for the streaming tab, and a rotated-away tab exits cleanly on its next `401`.
2. **Fail-open storage.** All token/settings persistence goes through a wrapper that catches quota/security exceptions and degrades to a session-only in-memory fallback; a storage failure is never misreported as a dead server.
3. **Bounded wait for server-gone.** A confirmed server-gone (resume validation, idle health check, streaming disconnect probe) enters a "Waiting for server…" state instead of locking: the stored token is re-validated every ~3s for up to ~60s, resuming with zero clicks when the server returns; on timeout it falls back to the ADR-0009 Reload lock. The QR/pair path is excluded (it keeps its soft retry). The wait tears down the stream and releases the microphone; the stream is never auto-resumed — the user taps the mic, matching the existing intent model.

The QR-clear behavior rests on a credential invariant: **PIN validity ⇔ token validity** — a credential reset (`--pin random` / deleted `pin` file) clears both, certificate rotation touches neither, and the sole exception (a manually deleted server-side token file) converges both flows to a single re-pair. Discarding the stored token when a QR hash is present therefore loses nothing.

## Reasoning

- **Validation without rotation is the entire fix for the multi-tab kick.** A renew on load hands the session to the reloading tab; validation on load proves the tab still holds a valid credential without moving ownership. The server needs no new endpoint: `/api/stats` already doubles as the 401 detector everywhere.
- **The wait replaces a lock whose premise died.** ADR-0009's Reload lock existed because every server start regenerated the certificate; ADR-0015 removed that, so the lock now fires on exactly the wrong cases (plain restarts). The client cannot tell a plain restart from a replaced machine (it cannot inspect its own TLS cert), so the wait is *bounded* — after the cap, the Reload lock remains the one recovery that also handles a changed certificate.
- **Fail-open storage is cheap and removes a whole failure class.** One wrapper, no behavior change when storage works.
- **Blocking resume** (inputs disabled, "Restoring session…") avoids the flash of a main screen that immediately bounces back to pairing on a stale token, and prevents a manual pair racing the validation.

## Alternatives considered (rejected)

- Keep renew-on-load + `storage`-event sync across tabs — rejected: sync keeps the loser alive with the synced token, so two live tabs can ping-pong through the handover machinery forever; a 401-terminated loser is cleaner.
- Validating via a new dedicated endpoint — rejected: `/api/stats` already provides the exact semantics (200 / 401 / 503-with-shutdown-precedence), and no wire change is worth it.
- Auto-resuming the stream when the wait ends — rejected: it drags in the mic-acquisition semantics of ADR-0003 (never grab the mic while hidden, no looped recovery) and changes the user-intent model; a tap is cheap.
- Extending the reconnect loop to cover the whole wait window — rejected for the same reason; the wait restores the *paired* state, the stream stays user-initiated.

## Consequences

- Opening or reloading a second tab no longer kicks a streaming tab (the defect fixed by this ADR).
- A plain server restart is now fully invisible on the client: the wait resumes into the main screen with zero clicks (previously: one manual Reload, stream stopped).
- Storage failures (private mode, blocked cookies) degrade to session-only pairing instead of dead-ending.
- The pairing screen gains two transient states: "Restoring session…" (resume validation) and "Waiting for server…" (bounded wait); the Reload lock remains, reachable only after the wait cap or via an explicit terminal.
- The `X-Session-Token` header / `/api/stats` endpoint now also serves as the resume validator; no server or wire-format change (ADR-0016 semantics stand).
- The wait state exits Eco Mode on entry (a dead server behind a black overlay must not look like a healthy idle screen).
- The pure decision logic (routing, verdicts, wait cadence, storage fallback) lives in `web/session.js`, unit-tested with Node's built-in runner; browser-coupled glue stays in `app.js`, verified on-device per the existing client precedent.

## Testing

- Unit tests (`web/session.test.js`, `node --test`, zero new dependencies): entry-routing matrix (hash/no-hash × token/no-token), HTTP verdict mapping (200/401/503/other/none), wait-cadence boundaries (~3s retry, ~60s cap), and safe-storage fail-open against throwing storage mocks — mirroring the "pure decision logic unit tests" pattern of ADR-0015/0016.
- On-device verification (repo client precedent, ADR-0005/0009): reload-resume; QR pair; second-tab non-kick; server restart mid-stream → wait → recover; certificate-replaced server → wait → Reload lock; incognito/private mode; Eco Mode exit; corrupt stored token.
- CI gates (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`) stay green; the JS tests run outside cargo.

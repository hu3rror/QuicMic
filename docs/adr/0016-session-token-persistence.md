# 0016 — Session token persistence and renewal across restarts

- **Status:** Accepted
- **Date:** 2026-09-16
- **Provenance:** grilling session → spec issue `hu3rror/QuicMic#2`; implemented in the session-token feature commit

## Background

The session token lived in memory only (`StreamState.session_token`), so a server restart destroyed it. With ADR-0015, a restart with the same LAN IP reuses the TLS identity and the PIN — but the in-memory token was gone, so `/api/renew` failed against the empty slot, `/api/stats` answered 401, and transports rejected the reconnect. The loaded client page had to re-enter the PIN even though nothing had changed: the ADR-0015 promise "a restart is invisible to the loaded page apart from the disconnect itself, no reload, no re-pair" held for the certificate and PIN, but not for the pairing credential half.

`/api/renew` itself already existed (ADR-0007): validate the presented token, issue a new one, broadcast cancellation, settle 50ms. This ADR extends its semantics — write-through plus restart survival — it does not add a new endpoint.

## Decision

Persist the session token as a third artifact in the data directory and seed the in-memory slot from it at startup.

1. **Third artifact** — a `session-token` file alongside `identity.json` and `pin`. Corruption recovery is independent per artifact: a corrupt session file is quarantined aside (`.corrupt-<ts>-<pid>`) and never touches the identity or PIN files, and vice versa (the ADR-0015 artifact model, extended by one).
2. **Content** — the plaintext token itself, exactly 64 lowercase hex characters (as `generate_hex_token` emits). No JSON, no metadata. A future expiry feature can upgrade to JSON compatibly. Strict validation on both write and read: wrong length, non-hex, uppercase hex, or surrounding whitespace (e.g. a text-editor trailing newline) is rejected on write and quarantined on read — a token file is machine-written, so leniency buys nothing.
3. **Write-through** — pairing and `/api/renew` persist the freshly issued token via the store's atomic-write pattern (sibling temp file, fsync, rename; mode 0600 on Unix). Renewal keeps its ADR-0007 behavior unchanged (constant-time validation, broadcast cancel, 50ms settle) and additionally writes the file.
4. **Startup load** — the boot path reads the file, validates strictly, and seeds the existing in-memory slot. Missing file → no token (silent; the normal first-run/unpaired state). Invalid file → warn + quarantine + no token. A token is **never regenerated from nothing**: "no token" is the honest unpaired state, exactly as a fresh server would present.
5. **PIN-reset linkage** — a boot that re-creates the pairing credential — explicit `--pin random`, or a missing `pin` file (which regenerates the PIN) — also clears the persisted token. `--pin <digits>` is **not** a reset: it writes the chosen PIN through and no regeneration happens, so the persisted token survives (the user is provisioning a specific credential, not revoking sessions). This is an **intentional exception to "independent lifecycles"**: that independence (ADR-0015) means corruption recovery, not credential semantics. PIN and session token are two halves of one pairing credential; a reset must revoke both, or the reset is theater (a kicked device's token renews forever). Note the boundary: a *corrupt* (not missing) `pin` file regenerates the PIN but deliberately does **not** clear the token — the user asked for nothing; the still-valid client keeps working, and any new pairing uses the new PIN.
6. **No TTL** — tokens never expire, matching pre-persistence semantics. Revocation is the reset action above or deleting the `session-token` file. A persisted token grants nothing more than the PIN does (anyone who can read the data directory can read either file, both protected identically), so expiry would only add "re-pair because you didn't reconnect within the window" friction on a trusted LAN.
7. **Semantics preserved** — every validation point (`/api/stats` header, `/api/settings` POST body, `/ws` query, WebTransport session, `/api/renew` body) still compares against the in-memory token. Pairing overwrites the token in memory **and** on disk, so another device's stale token gets 401 and the client re-pairs in place (ADR-0009's session-taken-over detection unchanged).

## Reasoning

- Persisting the token is the minimal completion of ADR-0015: the same restart that reuses the certificate and PIN now also reuses the pairing credential, so a plain restart is fully invisible to the loaded page. Renewal already runs first on every reconnect (the client renews to clear zombie sessions), so making it succeed after a restart requires no client change at all.
- Opaque-token persistence over signed/self-validating tokens: validation stays a constant-time comparison (no new crypto surface, no wire-format change, no clock handling), and the 401/session-taken-over semantics fall out of pairing-overwrites-token for free. On a LAN where the PIN is printed at boot, the signed-token advantages (expiry, sub-second revocation, stateless verification) buy nothing.
- Strict lowercase-hex validation mirrors the PIN's strict 6-digit validation: both artifacts are machine-written, both quarantine on corruption, and symmetric write/read rules make the round-trip contract testable.

## Alternatives considered (rejected)

- Persist a signing secret and issue signed self-validating tokens (with a generation counter to preserve the 401 takeover semantics) — rejected: real benefits (no per-reconnect disk write, TTL, revocation) not worth the crypto surface, wire-format change, and key/clock failure modes on a trusted LAN. Revisit only if token expiry or sub-second revocation ever becomes a requirement.
- Trim whitespace on read (like the PIN file does) — rejected: the PIN file is user-touched; the token file is machine-written, and the spec's strict contract treats whitespace as corrupt. A text-editor touch quarantining the file costs one re-pair; silently trimming would diverge write and read validation.
- Token TTL (e.g. 30 days) — rejected: no expiry today, and expiry only fires for clients that reconnect less often than the window — exactly the users for whom an unexplained re-pair is pure friction. `issued_at` metadata can be added compatibly if this ever changes.

## Consequences

- A plain restart (same LAN IP, no certificate rotation) keeps the loaded page paired: its next reconnect renews against the reloaded token. No reload, no re-pair.
- A PIN-reset boot (`--pin random`, or a missing `pin` file) revokes every paired client in one action; the 6-digit PIN is regenerated at the same time. Certificate rotation (IP change / 13-day age) never touches the token.
- A failed clear on a reset boot is logged and degrades to in-memory-only: that boot is safe (the slot is seeded `None`), but the stale file can be resurrected by a later plain restart until removed — the operator is warned and can delete the file manually. Tightening later could make the clear failure a startup error or retry once; not needed today.
- Write failures fail open: a failing `session-token` write on pair/renew logs a warning and keeps the in-memory token — the run continues in ephemeral-like mode, and the next restart may require one re-pair. Renewal is on the client's reconnect critical path, so a disk hiccup must not bounce the client to the pairing screen.
- Ephemeral mode (no usable data directory) is unchanged: no file IO, in-memory only.
- If PIN resolution fails on a boot that still has a working store (platform-default dir, `degrade` path), the boot uses an ephemeral PIN but still loads the persisted session token: a pre-restart client keeps its session (continuity beats a forced re-pair with a one-shot PIN), while new pairings use the ephemeral PIN until the store issue is fixed.
- Manual deletion of `session-token` while the server runs leaves the live session untouched (memory still holds the token); the file is rewritten at the next pair/renew. A restart before that reverts to "no token" → one re-pair.
- Single-instance convention stands (ADR-0015): atomic writes make concurrent pair/renew writes last-writer-wins at the file level; no locking added.

## Testing

Unit tests following the existing patterns (pure decision logic in `persistence.rs`; axum oneshot route tests in the server module; hand-rolled unique temp dirs, no new dev-dependencies):

- Store: token round-trip; missing file → no token and nothing created; invalid content (wrong length, non-hex, uppercase hex, trailing newline) → quarantined aside + no token; write rejects invalid values; clear removes the file and ignores a missing one; no `.tmp` leftovers; `pin_persisted` reflects the pin file; `init_session_token` loads on a plain boot, clears on a reset boot, and is `None` without a store.
- HTTP seam: pair write-through (returned token equals the persisted token); renew write-through with old-token invalidation; missing/wrong token → 401 (unchanged); restart-resume (persisted token → boot-path load → renew succeeds); PIN-reset boot (token cleared → renew fails); pairing takeover (device B pairs → device A's token → 401, store holds only B's token).
- CI gates (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`) stay green (ADR-0002).

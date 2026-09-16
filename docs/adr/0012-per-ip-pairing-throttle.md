# 0012 — Per-IP pairing throttle

- **Status:** Accepted
- **Date:** 2026-06-27
- **Provenance:** commit `643d4fe` (initial commit)

## Background

The `/api/pair` endpoint must resist brute-force PIN guessing without locking out legitimate users. An earlier version used a single global counter, which let one misbehaving host lock everyone else out.

## Decision

- **Per-IP rate limiting** on `/api/pair`: after **5 consecutive failed PIN attempts from the same client IP**, the endpoint returns `429 Too Many Requests` and locks out **that IP** for **30 seconds**.
- State lives in a single `Arc<parking_lot::Mutex<PairingThrottle>>`, where `PairingThrottle` holds a `HashMap<IpAddr, ThrottleEntry>`. Each IP's failure count and lockout deadline are read and updated together (the whole `handle_pair` bookkeeping runs in one synchronous critical section — no guard held across an `.await`). The attempted PIN is never logged.
- The client IP comes from `ConnectInfo<SocketAddr>` (HTTPS served with `into_make_service_with_connect_info`). A completed pair request requires a real TCP+TLS handshake, so the key cannot be spoofed to flood the map.
- **Idle pruning**: an entry is dropped once it is neither actively locked out nor seen within the `FAILURE_DECAY` window (60s), keeping the map bounded; a legitimate user's stray mistype decays away. Successful pairing clears that IP's entry.
- `PairingThrottle` has `#[cfg(test)]` unit tests (per-IP isolation, lockout, idle pruning).

## Reasoning

Keying by IP means one misbehaving host cannot lock everyone else out. Pruning keeps the map bounded without a background sweeper.

## Alternatives considered

- Global failure counter — rejected: one misbehaving host locks out everyone.
- No rate limiting — rejected: the PIN is 6 digits on an open LAN endpoint.
- Unbounded map — rejected: memory growth from spoofed sources over time.

## Consequences

- A mistyped PIN decays after 60s of inactivity; a burst of failures locks out only the offending IP for 30s.
# 0005 — Feature-detect the WebTransport datagram writer

- **Status:** Accepted
- **Date:** 2026-09-16
- **Provenance:** commit `b267299` (fork)

## Background

Safari/iOS 26.4+ only implements the modern `transport.datagrams.createWritable()`; the legacy `writable` property is **never exposed** there and accessing it throws a `TypeError`. The previous code accessed `writable` unconditionally, which made WebTransport silently fall back to WebSocket ~4ms after connecting (server log: `WebTransport client connected` → `connection closed by peer: 0`).

## Decision

Always feature-detect when acquiring the datagram writer — `createWritable()` first, legacy `writable` fallback (MDN BCD: `createWritable()` Safari 26.4+/Firefox 155+, legacy `writable` Chromium 97+/Firefox 114+). Never access either API without a capability check.

## Reasoning

The two APIs have disjoint browser support surfaces; unconditional access is a guaranteed `TypeError` on one of them.

## Alternatives considered

- Keep using `writable` only — rejected: throws on Safari/iOS 26.4+, the primary mobile target.
- Keep using `createWritable()` only — rejected: breaks Chromium 97–126-era engines that predate it.

## Consequences

- WebTransport stays active (rather than downgrading to WebSocket) across the full supported browser matrix.
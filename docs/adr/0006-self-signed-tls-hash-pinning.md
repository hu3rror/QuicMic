# 0006 — Self-signed TLS with serverCertificateHashes pinning on LAN

- **Status:** Accepted
- **Date:** 2026-06-27
- **Provenance:** commit `643d4fe` (initial commit)

## Background

WebTransport requires a secure HTTPS/QUIC connection. Local IP addresses (e.g. `192.168.1.X`) cannot easily get signed SSL certificates from public authorities (Let's Encrypt etc.), and the connection is LAN-only.

## Decision

1. Generate a self-signed ECDSA P-256 certificate on-the-fly (`src/tls.rs`) using `wtransport`'s internal builder **in memory**.
2. Compute the **SHA-256 hash** of the DER-encoded certificate using `ring`.
3. The client fetches the hash via `/api/info` and configures its WebTransport connection with `serverCertificateHashes`:
   ```javascript
   new WebTransport(url, {
       serverCertificateHashes: [{ algorithm: 'sha-256', value: hashBytes }],
   });
   ```
4. Certificate lifetime is **14 days** (the WebTransport spec maximum for `serverCertificateHashes`).
5. All operations are 100% in-memory with **zero disk writes** by default; `--dump-certs` exports PEM/DER to `certs/` for debugging.

## Reasoning

`serverCertificateHashes` lets modern browsers establish a secure UDP connection to a local IP without throwing certificate trust errors, at the cost of pinning the hash — which is exactly the right trust model for a per-boot server on your own LAN.

## Alternatives considered

- Public CA (Let's Encrypt) for a LAN IP — rejected: not issuable for IP literals without DNS control; overkill for LAN-only use.
- Ship a checked-in long-lived keypair — rejected: worse security posture (shared secret in every repo clone), no per-boot rotation.

## Consequences

- The certificate is regenerated on every server start, so a restarted server has a new hash — the loaded page's pinned hash is stale (drives the reload-required flow in ADR-0009).
- `--dump-certs` exists purely for debugging the pinning handshake.
# 0015 — Persisted identity (TLS keypair + pairing PIN) across restarts

- **Status:** Accepted
- **Date:** 2026-09-16
- **Provenance:** grilling session → issue `hu3rror/QuicMic#1`; implemented in the persisted-identity feature commit (supersedes ADR-0006 in part)

## Background

Every server start previously regenerated the TLS keypair in memory and drew a fresh random PIN (ADR-0006). Consequences: after a restart the loaded page's pinned hash is stale (drives the reload-required flow in ADR-0009) and every client must re-pair with a new PIN. For a machine that runs QuicMic at boot, every reboot is therefore a re-pairing event.

On a machine-local LAN none of that churn buys security: the PIN is printed to the console and in the QR at every boot anyway, and the LAN is trusted.

The W3C requirement behind ADR-0006 caps the *total validity period* of a certificate used with `serverCertificateHashes` at **two weeks** (wtransport's `self_signed` builder hard-codes 14 days), so a persisted certificate must be rotated before that boundary or browsers reject new connections.

## Decision

Persist the TLS keypair and the pairing PIN on the server machine, keyed to a per-machine data directory, so a restart with the same LAN IP reuses both.

1. **Data directory** — hand-rolled platform resolution (no `dirs` crate):
   - Windows: `%LOCALAPPDATA%\QuicMic\` (non-roaming — the private key must not roam with the profile; fall back to `%APPDATA%`, then the user home, if unset).
   - macOS: `~/Library/Application Support/QuicMic/`.
   - Linux: `$XDG_DATA_HOME/QuicMic`, else `~/.local/share/QuicMic`.
   - Precedence: `--data-dir <path>` CLI flag > `QUICMIC_DATA_DIR` env > platform default. The directory is created with `create_dir_all`.
   - An explicitly supplied `--data-dir` / `QUICMIC_DATA_DIR` that is unusable → **hard failure** (fail fast, clear error). An unusable *platform default* → warn and fall back to a one-shot in-memory identity for this run: the server still starts, nothing is persisted.

2. **Files** — `identity.json` (`cert_pem`, `key_pem`, `lan_ip`, `created_at`) plus a separate plaintext `pin` file. The two artifacts have independent lifecycles: rotation or corruption of the identity never touches the PIN and vice versa. All writes go to a temp file followed by an atomic rename in the same directory. Corrupt/unreadable files are renamed to `<name>.corrupt-<timestamp>` and regenerated, with a warning — the two artifacts recover independently (an identity failure does not discard a valid PIN).

3. **Load-or-generate** — at startup the server loads the persisted identity if present, otherwise it generates one and writes it. `lan_ip` records the *effective* IP the certificate was built for (the `--ip` override when given, else the detected LAN IP; the SAN list is `["localhost", ip]`). If the effective IP differs from the stored `lan_ip` — adapter/VPN changes after reboot are common — the certificate is regenerated for the new IP; the PIN is untouched.

4. **Rotation** — the certificate is regenerated (atomic in-place replacement of `identity.json`) when its age reaches **13 days**, keeping it comfortably inside the two-week W3C ceiling. Rotation happens **only at startup**: there is deliberately no runtime hot-swap of the live listeners. A process that stays up past the certificate's expiry will have new HTTPS/WebTransport connections rejected by browsers until it is restarted; restarting heals it (the client's existing reload flow, ADR-0009, applies).

5. **PIN** — first run generates a random 6-digit PIN and persists it; later runs reuse it. `--pin <6 digits>` validates, uses, and **persists** that PIN (write-through). `--pin random` regenerates and persists a fresh one; deleting the `pin` file has the same effect. The PIN is stored in plaintext: it is printed at every startup and is 6 digits, so hashing it at rest is theater.

6. **Permissions & trust model** — Unix: `identity.json` and `pin` are explicitly created with mode `0o600` (the default umask would otherwise yield `0644`; the mode is re-applied on rotation rewrites; setting modes is Unix-only — Windows relies on the user profile ACLs). No passphrase on the key: boot-time autostart requires zero interaction. Documented as LAN / trusted-network only.

7. **Single instance** — one QuicMic instance per machine is the documented convention. Atomic writes keep concurrent instances safe at the file level, but they intentionally share one identity and one PIN (and rotation races are not arbitrated by a lock).

## Reasoning

- The certificate hash pinning from ADR-0006 is the client's trust anchor; persisting the keypair keeps that anchor stable across restarts, so the restart is invisible to a loaded page apart from the disconnect itself (no reload, no re-pair).
- Startup-only rotation is the *free* reload moment: after a restart every client page already needs to reload per ADR-0009, so rotating then costs nothing new. A runtime hot-swap would instead rotate while pages are working — forcing a reload on healthy clients — the wrong trade for a machine that normally reboots well within 14 days.
- Zero new dependencies: platform directory resolution is a small, stable matrix; hand-rolled code is testable by injecting paths.

## Alternatives considered

- `dirs` crate for the data directory — rejected: a dependency for ~40 lines of platform handling; the hand-rolled version is unit-tested with injected paths (see Testing). Remains a viable fallback if the matrix ever grows.
- Runtime hot rotation (daily check + `RustlsConfig::reload_from_pem` + `Endpoint::reload_config`) — feasible and verified against the vendored wtransport 0.7.1 / axum-server 0.8 sources, but rejected for now: it forces a reload on healthy clients and requires hoisting `RustlsConfig` and `Endpoint` handles into shared state for live reconfiguration. If 7×24 uptime ever becomes a requirement, revisit in a new ADR.
- Filename-bucketed certificates per IP — rejected in favor of the `lan_ip` marker field: explicit, debuggable, one file.
- Hashing the PIN at rest — rejected: 6 digits, printed at boot; plaintext behind `0o600` / profile ACLs is honest protection.
- Checked-in long-lived keypair — rejected (as in ADR-0006): a machine key must not live in the repository.
- Per-boot ephemeral keypair (status quo, ADR-0006) — superseded in part by this ADR, which deliberately trades per-boot rotation for cross-restart stability on a trusted LAN.

## Consequences

- A server restart with the same LAN IP reuses the same self-signed certificate (same hash) and the same PIN. The reload-required consequence of ADR-0009 now applies only on certificate rotation (IP change, or age ≥ 13 days); after a plain restart it no longer applies.
- A process left running past the certificate expiry (≈2 weeks) serves certificates browsers reject until restarted. Documented; no runtime rotation.
- The private key now lives on disk in plaintext: whoever reads `identity.json` once can impersonate the pinned fingerprint indefinitely (LAN MITM), whereas the old in-memory key required presence at boot or process-memory access. Accepted for the LAN trust model; file permissions kept tight (Unix `0o600`).
- A changed LAN IP silently regenerates the certificate; the PIN never changes with it.
- `--dump-certs` keeps its meaning (export for inspection); the identity files additionally make the cert/key available on disk.
- The resampler safety net (ADR-0002) is untouched; the new unit tests cover the pure decision logic (see Testing) and the fmt/clippy/test gates stay green.

## Testing

Pure-logic unit tests following the `#[cfg(test)]` pattern in `src/server/state.rs`; no new dev-dependencies (unique temp dirs are hand-rolled under `std::env::temp_dir()`):

- Data-directory resolution and precedence (`--data-dir` > `QUICMIC_DATA_DIR` > platform default) and the env fallback chains.
- Rotation age decision (boundary at 13 days).
- PIN read/write, 6-digit validation, and `--pin random` regeneration.
- Corrupt/missing recovery: damaged `identity.json` → renamed `.corrupt-<ts>` + regenerated identity with the PIN file untouched, and the symmetric case (damaged `pin` → regenerated PIN, identity untouched).
- Identity round-trip: generate → persist → load → identical hash/PEM to the in-memory original; mismatched cert/key detected via the config-build validation and treated as corrupt.
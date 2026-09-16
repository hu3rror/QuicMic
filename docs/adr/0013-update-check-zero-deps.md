# 0013 — Update check with zero new dependencies

- **Status:** Accepted
- **Date:** 2026-06-27
- **Provenance:** commit `643d4fe` (initial commit)

## Background

A best-effort check for a newer GitHub release is useful, but adding an HTTP-client crate (and its transitive tree) to a deliberately lean dependency graph is not. The check must never block startup or fail loudly.

## Decision

- **No HTTP-client crate.** The check (`src/update_check.rs`) reuses the existing TLS stack (`rustls` + the process-default `ring` provider + `tokio-rustls`, all already in the tree) and the OS trust store (`rustls-native-certs`) to validate GitHub's public cert, and hits the `releases/latest` **redirect** — reading only the `Location` header (no response body or JSON to parse). The repo is `REPO` in the module. `tokio-rustls` is pinned `default-features = false` (+`ring`) so it never drags `aws-lc-rs` back in (see ADR-0014).
- Runs once at startup on a background `tokio::spawn` — **never blocks startup**, **silent on any failure** (offline, DNS/TLS error, unknown repo, malformed response).
- Only ever reports a *strictly newer* version (`parse_version` tuple comparison against `CARGO_PKG_VERSION`), so it cannot produce a false positive.
- Opt out with `--no-update-check` or `QUICMIC_NO_UPDATE_CHECK`.
- Result surfaced two ways: a one-line `info!` in the terminal, and `update_available` / `latest_version` / `releases_url` on `/api/info`. The web UI turns that into a small dismissible top banner linking to the releases page; the dismissal is remembered per version in `localStorage`. The browser never contacts GitHub (the server does), so the strict CSP (`connect-src 'self'`) is unchanged — an external `<a href>` navigation is not subject to CSP.

## Reasoning

The redirect's `Location` header *is* the answer (the latest version tag), so no body/JSON parsing and no HTTP client are needed. Reusing the existing TLS stack keeps the dependency graph intact.

## Alternatives considered

- Add an HTTP client crate — rejected: new transitive dependencies (possibly pulling `aws-lc-sys`, see ADR-0014) for a one-header request.
- Parse the releases JSON API — rejected: needs a body parser and API tolerance for a signal the redirect header already carries.
- Fail loudly on offline — rejected: silence on failure is the point of a best-effort check.

## Consequences

- The check is invisible unless it has real news.
- CSP stays strict; the banner is a plain link.
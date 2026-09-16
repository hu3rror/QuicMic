# 0011 — Hybrid static file serving with disk overrides and ETag revalidation

- **Status:** Accepted
- **Date:** 2026-06-27
- **Provenance:** commit `643d4fe` (initial commit)

## Background

The web UI must be customizable without recompiling the binary, while remaining fully portable as a standalone executable. Browsers (especially iOS Safari, for HTML and favicons) heuristically cached assets and never learned they had changed.

## Decision

- **Embedded assets (`rust-embed`)**: all files in `web/` are embedded into the binary at compile time.
- **Local disk overrides**: the fallback handler checks for a local `web/` directory in precedence order — (1) next to the executable (the documented customization location), then (2) the current working directory (covers `cargo run`, where the binary lives under `target/`). Only the directory *paths* are cached (`override_dirs` via `OnceLock` — the executable's location is fixed for the process); file existence and contents are re-checked on **every request**, so a `web/` file added/edited while the server runs is served immediately, with no restart.
- **Path-traversal guard (`is_safe_asset_path`)**: the request path is validated before any filesystem access — any path with a `..`/parent or absolute component is rejected with `404` (e.g. `GET /../Cargo.toml`). Browsers normalise `..` away, but a hand-crafted request would not, so the guard is required.
- **MIME guessing (`mime-guess`)**: correct `Content-Type` resolved for both local and embedded files.
- **Cache validation (`ETag` + `Cache-Control: no-cache`)**: every asset is served with an ETag and `no-cache`, so the browser revalidates on each load (`If-None-Match` → `304`) and picks up a rebuilt/edited asset immediately. ETag source differs by origin: **disk** overrides use a content-derived hash recomputed per request (so a same-length edit is detected for live editing); **embedded** assets use rust-embed's compile-time SHA-256 (no per-request hashing; body never materialized on a `304`).
- **Security & cache headers**: every served document carries a strict CSP (`default-src 'self'`; `img-src 'self' data:` for the inline favicon; `frame-ancestors 'none'`), plus `X-Frame-Options: DENY`, `X-Content-Type-Options: nosniff`, and `Referrer-Policy: no-referrer`. `style-src` is plain `'self'` with **no** `'unsafe-inline'`: all styling lives in `style.css` (no inline `<style>` blocks or `style="..."` attributes); JS-driven styling goes through the CSSOM (`element.style`), which CSP does not gate. A router-wide middleware (`apply_no_store`) adds `Cache-Control: no-store` to any response that did **not** set its own caching policy, so dynamic API / `/ca` responses are never heuristically cached.

## Reasoning

Hybrid serving gives customization without sacrificing portability. Revalidation (rather than `no-store`) keeps live editing and the revalidation the liveness/shutdown detection depends on working — the `no-store` middleware applies only where no policy is already set.

## Alternatives considered

- Serve only from disk — rejected: breaks standalone portability.
- Serve only embedded — rejected: no customization without recompiling.
- Long cache lifetimes — rejected: browsers (iOS Safari especially) never picked up rebuilt assets.
- `style-src 'unsafe-inline'` — rejected: unnecessary; all styling lives in `style.css` and CSSOM writes are not gated by CSP.

## Consequences

- Editing `web/` next to the executable is a live-reload experience (content-hash ETags catch same-length edits).
- Any future external resource load (or WebTransport/WebSocket breakage on iOS Safari) means widening the relevant CSP directive.
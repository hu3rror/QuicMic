# 0019 — In-app QR scan for pairing: vendored jsQR, scan ≡ typed PIN, progressive enhancement

- **Status:** Accepted
- **Date:** 2026-09-17
- **Provenance:** grilling session (G5) → design settled; implementation and acceptance follow as a separate step.

## Background

ADR-0018 verified on-device iOS storage isolation: the home-screen standalone window keeps its localStorage/cookies fully separate from Safari, so its *first* open requires one PIN pairing inside the standalone window. The OS-camera scan flow cannot help there — it deep-links into Safari and pairs Safari's storage, so the token lands in the wrong container and the user still has to type the PIN into the home-screen web app (or just type it from the start). The remaining friction of first pairing is that round-trip: open a separate camera app, scan, come back, type the PIN. An in-app scan makes the home-screen container's first pairing self-contained: scan → pair → token write-through into the standalone window's own storage partition → Resume from then on (ADR-0017).

## Decision

Add a scan entry to the pairing screen that decodes the server's existing Pairing QR with the page's own camera and feeds the PIN into the existing typed-PIN pair path. Pure `web/` change; the server is untouched.

1. **QR content unchanged.** The Pairing QR stays `https://<host>:<port>#<PIN>`: the OS-camera deep-link flow (hash route 'pair', ADR-0017) depends on the URL+PIN shape. The in-app scanner consumes the same artifact and extracts only `#(\d{6})$`; anything else is a scan failure, never an auth event. A scanned host/port differing from the current page's is informational only (the PIN is machine-pinned and does not rotate, ADR-0015; pair targets the current origin regardless).
2. **Decoder: vendored jsQR** (Apache-2.0, single file, ~100 KB minified) shipped with a license header. Not `BarcodeDetector` (unusable in Safari iOS — WebKit bug #281848 still open, iOS is the primary target) and not a self-written minimal decoder (day-scale effort and subtle misdecode risk for a 6-digit PIN that is already server-validated and has a manual fallback).
3. **Scan ≡ typed PIN.** The decoded PIN fills the existing input and runs the existing pair path: no `location.hash` mutation, no token clear, no history change. Pair is attempted exactly once per successful decode and never auto-retried, so a wrong-PIN scan cannot trip the 5-attempt/30s pairing throttle (ADR-0012) on its own. The QR-hash route keeps its QR-clear semantics for OS deep-links only.
4. **Permission flow: two separate prompts.** Scanning requests `getUserMedia({ video: { facingMode: 'environment' } })` at button tap (one camera prompt); the mic keeps its existing prompt at connect time. No combined `{audio, video}` request: that would move mic acquisition earlier into the pairing screen and disturb the audio-graph lifecycle (ADR-0003's foreground-only acquisition and recovery ground truths).
5. **Progressive enhancement, never a gate.** Manual PIN entry always remains. Camera denial/unavailability — including the historically unreliable iOS standalone PWA camera (`navigator.mediaDevices` missing or repeated prompts) — degrades to a clear message plus manual entry. The feature never gates pairing.
6. **Camera lifecycle.** Live viewfinder loop on a downscaled canvas (~5–10 fps); the camera track is stopped on success, cancel, or page-hide; `NotAllowedError` → manual-entry hint, other capture errors → retry hint. No CSP or Permissions-Policy header change (`media-src` does not govern `srcObject` capture streams — verify on all three target browsers during implementation).

## Reasoning

- The honest value statement is "免手输 PIN": G5 removes the *first-pairing round-trip* of the storage-isolated home-screen entry (and of any first open); it does not remove the handshake — Resume (ADR-0017) already removes that once paired.
- Scope stays minimal because every pairing-affecting flow already converges: session take-over and credential reset land on the same pairing screen, so the scan button is available everywhere pairing happens without introducing a third pairing mode.
- Wrong-PIN behavior needs no new handling: `/api/pair` answers `200 { success: false, "Incorrect PIN" }` (and 429 after five failures, ADR-0012) — identical for scanned and typed PINs, so in-app scan adds no auth surface: the page decodes a PIN the user could equally see and type.
- Vendoring one well-known Apache-2.0 file preserves the repo's no-build-chain, self-contained web stack (plain classic scripts, zero npm) — the frontend analogue of the zero-dependency stance in ADR-0013, with the vendor file keeping its attribution header.
- iOS standalone camera access is accepted as a documented known-risk, characterized during real-device acceptance: even where a given iOS version breaks PWA camera capture, Safari and Android Chrome still benefit, and the pairing flow itself is unaffected.

## Alternatives considered

- **PIN-only QR** — rejected: breaks the OS-camera deep-link flow, which is the primary QR consumer.
- **BarcodeDetector (native)** — rejected: not available in Safari iOS (WebKit bug #281848), the primary target.
- **Self-written minimal QR decoder** — rejected: cost and subtle-failure risk disproportionate to a server-validated credential with a manual fallback; a misdecode costs one failed pair attempt.
- **Single-shot photo scan** — rejected: worse UX (shutter + precise framing) than a live viewfinder for negligible CPU savings.
- **Combined {audio, video} permission prompt at pairing** — rejected: moves mic acquisition earlier and changes the existing audio-graph lifecycle.
- **Scan with QR-hash semantics (clear token + set hash)** — rejected: the pairing screen implies no valid token anyway and `doPair` write-throughs the new one; typed-PIN semantics keeps exactly one pairing mode.
- **A second PIN-only QR printed next to the URL QR** — rejected: terminal clutter with no benefit.

## Consequences

- `web/` gains: a vendored jsQR file (single file + license header), a pure `qr` module (text parsing + scan orchestration, UMD structure like `session.js`), the scan button and camera overlay in `index.html`, and glue in `app.js`.
- CSP is unchanged; the `srcObject` capture stream is verified against `media-src` fallback (`default-src 'self'`) on Safari, Chrome, and the iOS standalone container during implementation.
- First pairing now shows two permission prompts (camera at scan, mic at connect); users who type the PIN see no camera prompt.
- Storage behavior is unchanged: a scan performed inside the home-screen window writes the token into that window's own partition, so the subsequent Resume works — the point of G5.
- Test seams follow repo convention (CI stays cargo-only): node unit tests for the parse function; one decode integration test using a committed raw-pixel (PPM) fixture of a real QR through the vendored jsQR; manual acceptance on iOS Safari, iOS home-screen standalone, and Android Chrome scanning a live terminal QR.
- Docs: CONTEXT.md gains "Pairing QR / 配对二维码" and "In-app scan / 应用内扫码".
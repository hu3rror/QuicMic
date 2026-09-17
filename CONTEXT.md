# QuicMic — Domain Model

## What this project is

QuicMic turns a mobile or desktop device's web browser (iOS Safari, Android Chrome, desktop browsers, etc.) into a low-latency wireless PC microphone over a Local Area Network (LAN):

- **Client** — an HTML5/JS web page that captures mic input via `getUserMedia`, processes it in an `AudioWorklet`, and streams it to the server.
- **Server** — a Rust server (Axum + WTransport) hosting the web assets, validating client pairing, receiving raw audio packets, and playing them into a virtual audio device (e.g. VB-Cable).

## Glossary

Use these terms as defined here when naming domain concepts in issues, specs, tests, or refactors (see `docs/agents/domain.md`).

- **Client / phone** — the browser page capturing and streaming audio. The primary mobile target is iOS Safari (also Android Chrome, desktop browsers).
- **Server / PC** — the Rust binary running on the target machine, hosting the UI, pairing, transports, and the audio output device.
- **Pairing** — the PIN-based handshake by which a client proves it may stream (6-digit PIN shown in the startup QR; see ADR-0006, ADR-0012, ADR-0015). A paired client holds a **session token**.
- **Pairing QR / 配对二维码** — the QR printed at server startup, encoding `https://<host>:<port>#<6-digit PIN>`. One artifact, two consumers: the OS camera (deep-link into Safari via the `#PIN` hash route) and the in-app camera (PIN extraction for the same-origin pair, ADR-0019). Its content must stay `URL#PIN` — a PIN-only code would break the OS-camera flow.
- **In-app scan / 应用内扫码** — pairing whose PIN is obtained by decoding the Pairing QR with the page's own camera instead of typing it or using the OS camera. Semantics are identical to typing the PIN — a full handshake, never Resume; no hash mutation, no token clear, pair attempted once per decode (ADR-0019). Purpose: make the storage-isolated home-screen container's first pairing self-contained (ADR-0018).
- **Persisted identity / 持久身份** — the server's machine-local pairing material reused across restarts: the TLS self-signed keypair (certificate + private key) together with the 6-digit pairing PIN. It lives in the platform data directory (see ADR-0015) and is tied to the LAN IP it was built for. The certificate keypair and the PIN are independent artifacts with independent lifecycles: the certificate rotates, the PIN does not.
- **Identity rotation / 身份轮换** — regeneration of the server's TLS certificate keypair only, triggered when the LAN IP changed or the certificate approaches the two-week validity ceiling (≤ 13 days). The pairing PIN never changes with rotation; deleting the `pin` file or passing `--pin random` is the only way to reset it.
- **Session token** — the per-session secret renewed by `/api/renew` on every (re)connect, carried on `/api/stats`, `/api/settings` POST, `/api/client-state`, `/ws`, and the WebTransport session. It is persisted across server restarts (alongside the PIN, ADR-0016), so a plain restart keeps the loaded page paired; a credential reset clears it. The paired client mirrors it in browser storage so a page reload resumes without re-entering the PIN (see **Resume**). See `docs/architecture.md` → Auth surfaces.
- **Resume / 恢复会话** — the client-side counterpart of Pairing: re-entering the main screen from a stored session token, validating it without rotating it, and without the PIN handshake.
- **Home-screen entry / 主屏入口** — the standalone window opened from a home-screen icon (iOS "Add to Home Screen", Android manual add). Its `start_url` carries no URL hash, so it routes through Resume: a paired device opens straight into the main screen with no PIN (ADR-0018).
- **Credential reset / 凭证重置** — the act of revoking all pairing: regenerating the PIN (`--pin random`) or deleting the `pin` file also clears the persisted session token, so one action invalidates every paired client (ADR-0016). Certificate rotation is not a credential reset — it never touches the PIN or the session token.
- **Stream** — one active audio connection. The server allows exactly **one** at a time (single-connection contract, ADR-0007).
- **Transport** — the network path for audio: **WebTransport** (QUIC/UDP, primary) or **WebSocket** (TCP, fallback). Transport-agnostic logic must not branch on close codes (ADR-0009).
- **Packet / frame** — one network unit: 4-byte little-endian **sequence number** + raw i16 PCM samples (≤ 480 samples = 10ms @ 48kHz).
- **Sequence number** — monotonically increasing per *sent* packet only; gated-out silence never consumes one, keeping the stream contiguous for loss tracking (ADR-0008).
- **Ring buffer** — the lock-free SPSC buffer (`src/audio/ring_buffer.rs`) bridging transports and the output thread. Exactly one producer, one consumer (SPSC contract).
- **Noise gate / gate** — client-side DSP that withholds silence from the wire (250ms hold). The gate keeps the phone's radio idle during silence. "No packets" during silence is correct, never a fault (ADR-0003, ADR-0008).
- **Gain** — client-side float-domain amplification applied before Int16 conversion.
- **Latency recovery / hard skip** — server-side trim of ring-buffer lag down to the prebuffer mark (see `docs/architecture.md`); driven by `latency_threshold`.
- **Prebuffer** — the ~30ms low-water mark (`PREBUFFER_MS`) the output stage targets.
- **Mute** — long-press (500ms) on the mic button; the client stops sending audio while the worklet keeps running and heartbeating.
- **Eco Mode** — a solid black fullscreen overlay (OLED off) with wake-lock and suspended stats UI to save mobile battery.
- **Disconnect / server-gone** — a definitive "server left" verdict, detected over HTTP (`503` / unreachable), *never* from a transport close event (ADR-0009).
- **Reconnect** — transient-drop recovery with exponential backoff, renewing the session token per attempt.
- **Audio health** — `FLOW ∧ FLAGS`: the worklet is actually posting **(flow)** **and** the track/context flags read live/running **(flags)**. The single most important client invariant (ADR-0003).
- **VU / level** — the metering signal driven by the worklet's throttled `{ level }` messages.
- **LossTracker** — the reorder-tolerant sliding-window loss counter on the WebTransport path (`docs/architecture.md` → Statistics).

## Where the decisions live

- `docs/architecture.md` — how the system works today: audio pipeline, packet format, client mechanisms, statistics, module map, auth surfaces.
- `docs/adr/` — one file per design decision and its rationale (numbered `00NN-*.md`, newest last). Read the ones touching the area you're about to modify. Flag any contradiction with an existing ADR explicitly rather than silently overriding it.
- This file is the starting point; `docs/architecture.md` is the mechanism reference; the ADRs are the *why* behind the mechanisms.

## Signal for /domain-modeling

If a term you need is not in this glossary (or a definition here has drifted), that's a real gap — flag it for `/domain-modeling` rather than silently inventing synonyms.
# 0014 — Crypto provider pinned to `ring` (no aws-lc-rs)

- **Status:** Accepted
- **Date:** 2026-06-27
- **Provenance:** commit `643d4fe` (initial commit)

## Background

The whole dependency tree must build on the local `x86_64-pc-windows-gnu` toolchain (w64devkit-style, no MSVC). `aws-lc-rs`/`aws-lc-sys` need a C compiler and NASM, which would reintroduce that build burden. Rustls' default features pull `aws-lc-rs` in silently.

## Decision

The whole tree is pinned to the **`ring`** crypto provider:

- `rustls` is pulled with `default-features = false, features = ["ring", ...]`.
- `axum-server` with `tls-rustls-no-provider` (we install the ring provider ourselves in `run`).
- `tokio-rustls` pinned `default-features = false` (+`ring`).
- This deliberately keeps `aws-lc-rs`/`aws-lc-sys` out of the dependency graph.
- Do **not** re-add a default-featured `rustls`/`axum-server` (`tls-rustls`) — it would silently pull `aws-lc-sys` back in and reintroduce the C/NASM build burden.

## Reasoning

Ring builds everywhere the project targets without extra C/NASM tooling. The trade-off is no post-quantum key exchange, which `ring` does not implement and a LAN self-signed setup does not need (ADR-0006).

## Alternatives considered

- Default-featured rustls (aws-lc backend) — rejected: requires C compiler + NASM, breaks the gnu toolchain build.
- Per-platform feature gating — rejected: more matrix complexity for a LAN app with no PQ requirement.

## Consequences

- `cargo build` works on the w64devkit gnu toolchain with no extra system tooling.
- Any future dependency that would pull a default-featured rustls must be pinned the same way.
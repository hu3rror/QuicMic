# 0004 — Proxy-TUN fake-ip filtering in LAN IP detection

- **Status:** Accepted
- **Date:** 2026-09-16
- **Provenance:** commit `95f8302` (fork; PR Fix3dll/QuicMic#3, final form `78aed03` after upstream review)

## Background

`local_ip_address::local_ip()` returns the first IPv4 unicast address among the adapters that own a *default route*. When a proxy TUN in fake-ip mode (mihomo/Clash, sing-box, Surge, etc. — e.g. `device: sekai` holding `198.18.0.1`) owns a default route with a lower metric, its fake address wins — and nothing on the LAN can route to it. QuicMic printed and bound a URL that was unreachable.

## Decision

- `detect_lan_ip()` trusts the default-route pick **only when it is a usable LAN address**. `is_unusable_lan_addr` rejects `0.0.0.0/8`, loopback, link-local, multicast/reserved `240/4`, and the RFC 2544 benchmark range `198.18.0.0/15` (the de-facto fake-ip default for mihomo/Clash, sing-box, Surge).
- Otherwise `list_afinet_netifas()` is scanned and the best-ranked candidate is picked (`pick_lan_ip` + `lan_addr_rank`): RFC 1918 (2) > CGNAT `100.64/10` and IPv6 (1) > public IPv4 (0), IPv4 over IPv6 on rank ties. Same-rank same-family ties (e.g. Wi-Fi `192.168.x` vs a Hyper-V/WSL `172.x`) break in favour of physical adapters: virtual ones (`is_virtual_adapter`: `vEthernet`/`wsl`/`docker`/`vmnet`/`tun`/… name patterns) are deprioritized, and any remaining tie resolves by OS enumeration order (`max_by_key`'s last maximum).
- `--ip` remains the manual override.
- The pure functions carry unit tests.
- Behavior change bundled in the same commit: `detect_lan_ip` swallows `local_ip()` errors and falls through to the full scan, so an APIPA-only machine fails cleanly after the scan instead of on the default-route pick. (The initially-bundled `chunks_exact` → `as_chunks` clippy fix for `decode_into_ring` was dropped from the PR: upstream fixed the same lint itself in `4c4e14f`, so the processor change no longer exists beyond upstream `main`.)

## Reasoning

Verification against the sing-box and Surge official docs confirmed all major fake-ip proxy TUNs default to `198.18.0.0/15`. The default-route pick is correct in the common case, so it is kept and gated rather than abandoned.

## Alternatives considered

- Always scan all interfaces instead of trusting the default-route pick — rejected: the default-route pick is right for normal machines and matches user intent (the "real" uplink).
- Never fall through on `local_ip()` error — rejected: an APIPA-only machine would fail on the pick instead of the scan.

## Consequences

- The printed pairing URL is reachable on the LAN even with a fake-ip proxy TUN active.
- Same-rank ties prefer physical adapters over virtual ones; any remaining tie stays OS-enumeration-order dependent. `--ip` is the deterministic override.
# Verification

Planned checks:

- Rust RemoteApp network/view tests cover client ICE server projection.
- Frontend RemoteApp store tests prove `createPeerConnection` receives
  session-projected ICE servers.
- Static gates reject frontend regressions to hard-coded empty ICE config.
- Product readiness docs continue to mark NAT/relay as partial, not complete.

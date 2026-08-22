# Evidence Audit — RemoteApp Product Closure

Authoritative product readiness source:

- `docs/design/remoteapp-product-readiness-audit-2026-08-22.md`

Current conclusion:

- Targeted-session architecture: implemented with source and host-E2E harnesses.
- Full interactive RemoteApp product: incomplete.

Current verified boundary gates:

- `check-remoteapp-target-binding-boundary.sh`
- `check-remoteapp-lifecycle-input-boundary.sh`
- `check-remoteapp-e2e-acceptance-boundary.sh`
- `check-remoteapp-frontend-invocation-boundary.sh`
- `check-remoteapp-performance-boundary.sh`
- `check-remoteapp-picker-subject-boundary.sh`
- `check-remoteapp-session-subject-boundary.sh`

Current frontend lifecycle evidence:

- Frontend `media-channel-store` starts `remote_desktop.watch_events` after
  negotiated WebRTC setup with the selected target subject, session token, and
  consent causal context.
- Frontend unit coverage proves degraded session events surface a
  retry-session state and permission-revoked events close local WebRTC/input
  transport.
- `check-remoteapp-frontend-invocation-boundary.sh` now gates both the
  watch_events subscription and the recovery-event consumption contract.

Missing or insufficient product evidence:

- Cross-platform capture implementation/evidence for Windows and Linux.
- Real input injection E2E for pointer/keyboard.
- Audio path and codec/adaptation soak reports.
- Multi-display application capture or explicit product unsupported flow.
- Session resume/reconnect/revoke/crash-restart recovery E2E.
- Real STUN/TURN/EasyNet relay reachability matrix.
- Frontend full lifecycle E2E across Browser/Tauri surfaces.
- Cross-device smoke/regression with remote target inventory and teardown.

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
- `check-remoteapp-frontend-product-flow-e2e.sh`

Current frontend lifecycle evidence:

- Frontend `DeviceMediaAccess` component coverage drives the user-visible
  Remote desktop flow from target picker through Share, target-scoped consent,
  `create_session`, WebRTC signaling, `watch_events`, and End.
- Frontend `media-channel-store` starts `remote_desktop.watch_events` after
  negotiated WebRTC setup with the selected target subject, session token, and
  consent causal context.
- Frontend unit coverage proves degraded session events surface a
  retry-session state and permission-revoked events close local WebRTC/input
  transport.
- `check-remoteapp-frontend-invocation-boundary.sh` now gates both the
  watch_events subscription and the recovery-event consumption contract.
- `tools/scripts/frontend-remoteapp-product-flow-e2e.sh` now provides the
  combined frontend/host product-flow harness entrypoint: explicit product
  runtime readiness preflight, frontend typecheck, `DeviceMediaAccess` UI flow,
  host permission-subject preflight, target picker freshness, decoded-frame
  WebRTC, and view-only input safety. An explicit --run report remains required
  before treating it as environment evidence; the default skipped
  report only proves the harness contract exists.
- 2026-08-22 local `--run` attempt reached frontend typecheck and
  `DeviceMediaAccess` UI flow successfully, then failed before host RemoteApp
  execution because daemon readiness was false:
  `runtime_status=projection_present_process_missing`,
  `daemon.control_accepting=false`, `daemon.invocation_accepting=false`,
  `daemon.pid_alive=false`, and connection failure
  `START_FAILED_CREDENTIAL_VERIFY: Hub credential verification is unavailable`.
  This is environment evidence against product completion, not a RemoteApp
  pass.
- The connection-state snapshot now carries both the Hub session endpoint and
  credential-verification API endpoint. The current local report names
  `hub_endpoint=https://127.0.0.1:50443` and
  `hub_api_endpoint=http://localhost:8080`; the API endpoint is refusing
  connections because the local Hub/Docker runtime is not running. RemoteApp
  product E2E must not proceed to host capture/media/input evidence until this
  upstream product readiness gate is green.
- The product-flow harness now executes that upstream product runtime
  readiness preflight before frontend and host evidence. This preserves the
  product semantics: a failed Hub API / daemon invocation gate is the first
  failure, not a later host permission or media-capture symptom.
- Latest local `--run` after the order fix fails fast at
  `product-runtime-readiness-preflight` only, with
  `hub_api_endpoint=http://localhost:8080` and no frontend/host evidence steps
  recorded. That is the correct current product evidence shape while the local
  Hub API remains unavailable.

Missing or insufficient product evidence:

- Cross-platform capture implementation/evidence for Windows and Linux.
- Real input injection E2E for pointer/keyboard.
- Audio path and codec/adaptation soak reports.
- Multi-display application capture or explicit product unsupported flow.
- Session resume/reconnect/revoke/crash-restart recovery E2E.
- Real STUN/TURN/EasyNet relay reachability matrix.
- Frontend full lifecycle E2E across Browser/Tauri surfaces.
- Cross-device smoke/regression with remote target inventory and teardown.

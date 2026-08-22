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
  combined frontend/host product-flow harness entrypoint: explicit Hub API
  readiness preflight, product runtime readiness preflight, frontend typecheck,
  `DeviceMediaAccess` UI flow, host permission-subject preflight, target picker
  freshness, decoded-frame WebRTC, and view-only input safety. An explicit --run report remains required
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
- `tools/scripts/hub-api-readiness-preflight.sh` now isolates the first upstream
  product gate: runtime status must expose the Hub API endpoint, Docker must be
  reachable, and `${hub_api_endpoint}/api/v1/health` must respond before daemon,
  frontend, host capture, media, or input evidence can run.
- 2026-08-22 runtime diagnosis found two upstream product-readiness failures
  before RemoteApp evidence could be trusted:
  - Docker was initially unavailable, then recovered after Docker Desktop
    started.
  - The Hub compose default `HUB_REALM=easynet.run` conflicted with persisted
    `localhost` hosted-Agent inventory rows. Restarting the existing
    `easynet-dev` Hub with `HUB_REALM=localhost HUB_HTTP_PORT=8080` restored
    `/api/v1/health` for the paired local credentials.
- The device session connection-state projector now preserves the prior
  `hub_api_endpoint` when it promotes the read model to `FRONTEND_CONNECTED`.
  Without that fix, the running state dropped the Hub API endpoint that
  failure states exposed, and the product-flow harness could not deterministically
  perform the Hub API readiness gate after daemon recovery.
- Latest local product-flow evidence:
  `target/e2e/frontend-remoteapp-product-flow/20260822-044248-69775/report.md`
  passed all bounded local steps:
  Hub API readiness, product runtime readiness, frontend typecheck,
  `DeviceMediaAccess` UI flow, host permission-subject preflight, target picker
  freshness, decoded-frame WebRTC for window and application targets, and
  view-only input safety for window and application targets. This is strong
  local product-flow evidence, not cross-platform/cross-device product
  completion evidence.
- `tools/scripts/remoteapp-cross-device-product-smoke.sh` now provides a
  separate cross-device product smoke entrypoint. With `--run`, it composes
  the existing Docker two-node EasyRemote CLI routing E2E and Docker synthetic
  media/bidi E2E under one report. The report marks cross-device Hub routing
  and synthetic stream/bidi carrier coverage separately, and keeps real OS
  capture, pointer/keyboard injection, host audio, NAT/STUN/TURN relay
  deployment, and frontend rendering as non-claims.
- Latest local cross-device `--run` evidence:
  `target/e2e/remoteapp-cross-device-product-smoke/20260822-044924-manual/report.md`
  failed at `cross-device-routing` before synthetic media/bidi could run. The
  provider joined and was visible as an online federated device, but the caller
  device repeatedly failed the user-scoped Service owner projection prelude for
  `easynet:///r/hub/service/alice.pages`: Hub rejected
  `federation.advertise_abilities` with `accepted_count=0, expected_count=5`.
  This is now the first concrete cross-device product seam to fix before
  RemoteApp-specific remote target inventory/media evidence can be trusted.
- 2026-08-22 follow-up diagnosis: that failure is a Hub owner-projection
  read-model conflict, not an authority rejection. The Hub stores one selected
  projection per `owner_ura`; when two devices for the same user publish
  `service/<user>.pages` with the same generation/revision but different
  host/digest, the second write is a `rejected_conflict`. That Service surface
  must remain on the already-selected host, but the caller Device session must
  not reconnect/backoff and appear offline because RemoteApp device-native
  abilities are independent SystemAgent descriptors. The fix is to expose the
  Hub projection upsert `outcome` and let the user-scoped Service prelude
  degrade only on read-model rejection while keeping device-native and
  hosted-agent projections strict.
- 2026-08-22 verification after the Service projection fix:
  response/unit/script gates passed, but an actual
  `remoteapp-cross-device-product-smoke.sh --run` attempt did not produce
  authoritative product evidence. The child routing script blocked in
  `docker info`; after interruption, the harness could not write `result.json`
  because the local volume was full from regenerated Rust build artifacts.
  This is external environment evidence only. It does not contradict the unit
  fix, and it does not prove cross-device product readiness.
- The cross-device product smoke harness now fails before child E2Es with a
  structured report when the report filesystem lacks sufficient free space or
  when `docker info` hangs/fails. Each child E2E is also bounded by a step
  timeout. This keeps cross-device evidence auditable: environment failures
  remain failed reports with explicit reasons instead of indefinite hangs or
  missing `result.json` files.
- Latest local structured environment report:
  `target/e2e/remoteapp-cross-device-product-smoke/20260822-051119-57565/report.json`
  failed before child E2Es with reason `docker info timed out after 3s` and
  both cross-device routing and synthetic media coverage marked false.
- `docs/design/remoteapp-product-readiness-matrix.json` now records the
  machine-readable product closure state for the eight explicit requirements:
  application/window capture, input injection, audio/video adaptation,
  multi-window tracking, session recovery lifecycle, network fallback,
  frontend lifecycle, and cross-device E2E. The product closure audit gate
  rejects missing rows, unsupported statuses, empty evidence fields, and any
  premature `product_complete=true` claim.
- RemoteApp session views now expose `input_readiness` as a single
  machine-readable projection for requested mode, effective mode,
  `interactive_ready`, input scope, and blocked reason. This improves frontend
  and E2E diagnosability for the input-injection row, but the row remains
  incomplete until real focus-safe pointer/keyboard injection and latency
  evidence exists.
- Frontend protocol projection now parses daemon `input_readiness` and input
  sending prefers that runtime readiness over legacy `input_policy`. If the
  daemon reports `interactive_ready=false`, pointer/key frames fail closed
  before transport send. This closes the UI gating seam for requested
  interactive sessions that are correctly downgraded to view-only, while still
  leaving real OS input injection product evidence incomplete.
- RemoteApp consent now separates media/session consent from input-control
  consent. `grant_consent` may mint an explicit `input_control` scoped ticket;
  `create_session` consumes that scope before target binding resolution. Only
  display targets with explicit input-control consent can project
  `display_global` input scope. Window/application targets remain view-only
  because target-scoped keyboard/pointer dispatch still lacks the required
  focus/activation proof. Missing macOS Accessibility permission still reports
  `input_injection_unavailable` in `input_readiness`, so this is a consent and
  policy closure, not successful input-injection E2E evidence.
- Frontend RemoteApp creation now sends the same input intent through
  `grant_consent.args.input_control`, `create_session.args.mode`, and
  `create_session.args.input_policy`. The default Interactive path requests
  `input_control=true`; explicit view-only requests carry `input_control=false`
  and disabled keyboard/pointer policy. The CLI frontend boundary gate now
  rejects drift away from this shared-intent contract.

Missing or insufficient product evidence:

- Cross-platform capture implementation/evidence for Windows and Linux.
- Real input injection E2E for pointer/keyboard.
- Audio path and codec/adaptation soak reports.
- Multi-display application capture or explicit product unsupported flow.
- Session resume/reconnect/revoke/crash-restart recovery E2E.
- Real STUN/TURN/EasyNet relay reachability matrix.
- Frontend full lifecycle E2E across Browser/Tauri surfaces.
- RemoteApp-specific cross-device smoke/regression with remote target
  inventory, real display/window/application capture, input policy, and
  teardown.

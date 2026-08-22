# Evidence Audit — RemoteApp Product Closure

Authoritative product readiness source:

- `docs/design/remoteapp-product-readiness-audit-2026-08-22.md`

Current conclusion:

- Targeted-session architecture: implemented with source and host-E2E harnesses.
- Full interactive RemoteApp product: incomplete.
- RemoteApp implementation test evidence must come from the main EasyNet crate,
  not the standalone `easynet-plugin-remote-desktop` package. The standalone
  package is a provider/export shim whose zero-test result does not exercise
  the daemon-embedded implementation.

Current verified boundary gates:

- `check-remoteapp-target-binding-boundary.sh`
- `check-remoteapp-lifecycle-input-boundary.sh`
- `check-remoteapp-e2e-acceptance-boundary.sh`
- `check-remoteapp-frontend-invocation-boundary.sh`
- `check-remoteapp-performance-boundary.sh`
- `check-remoteapp-picker-subject-boundary.sh`
- `check-remoteapp-session-subject-boundary.sh`
- `check-remoteapp-frontend-product-flow-e2e.sh`
- `check-remoteapp-main-crate-implementation-tests.sh`

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
- `tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh` now provides the
  Browser/Tauri lifecycle evidence verifier. It accepts evidence from a real UI
  runner and requires `real_browser_tauri_lifecycle`, `component_mock=false`,
  `real_backend_runtime=true`, ordered picker/permission/consent/create/attach/
  watch/media/input/end/terminal-receipt steps, public RemoteApp ability names,
  host-local `permission_status`, selected Resource URA subject binding for
  session abilities, and no product-complete claim. Self-test validates only
  the contract; a live Browser/Tauri artifact remains required.
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
- macOS ScreenCaptureKit application sessions now build the native
  `exceptingWindows` filter from same-application, same-display windows outside
  the committed `AppWindowSetProof`. This closes a concrete capture-scope seam:
  selecting a committed application window set should not widen to unrelated
  same-app windows that appear after session creation. It remains macOS-local
  source/gate evidence, not cross-platform or real churn E2E completion.
- `tools/scripts/remoteapp-cross-platform-capture-e2e.sh` now provides the
  cross-platform capture evidence verifier. It accepts evidence from real
  macOS/Windows/Linux host runners, requires macOS display/window/application
  capture to pass with rendered frames and exact target binding, allows Windows
  and Linux to pass capture or report explicit product unsupported state, and
  rejects window/application display fallback, source-only proof, missing public
  RemoteApp session abilities, missing selected Resource URA subject binding,
  and missing visible terminal receipts. Self-test validates only the contract;
  a live platform artifact remains required.
- `tools/scripts/remoteapp-cross-device-product-smoke.sh` now provides a
  separate cross-device product smoke entrypoint. With `--run`, it composes
  the existing Docker two-node EasyRemote CLI routing E2E and Docker synthetic
  media/bidi E2E under one report. The report marks cross-device Hub routing
  and synthetic stream/bidi carrier coverage separately, and keeps real OS
  capture, pointer/keyboard injection, host audio, NAT/STUN/TURN relay
  deployment, and frontend rendering as non-claims.
- Historical local cross-device `--run` evidence:
  `target/e2e/remoteapp-cross-device-product-smoke/20260822-044924-manual/report.md`
  failed at `cross-device-routing` before synthetic media/bidi could run. The
  provider joined and was visible as an online federated device, but the caller
  device repeatedly failed the user-scoped Service owner projection prelude for
  `easynet:///r/hub/service/alice.pages`: Hub rejected
  `federation.advertise_abilities` with `accepted_count=0, expected_count=5`.
  This was the first concrete cross-device product seam blocking RemoteApp-
  specific remote target inventory/media evidence.
- 2026-08-22 follow-up diagnosis: that failure was a Hub owner-projection
  read-model conflict, not an authority rejection. The read model then stored
  one selected projection per `owner_ura`; when two devices for the same user
  published `service/<user>.pages` with the same generation/revision but
  different host/digest, the second write was a `rejected_conflict`. That
  diagnosis led to the Service owner multihost fix below. Device-native
  RemoteApp abilities remained independent SystemAgent descriptors and must not
  be taken offline by a user Service projection conflict.
- RemoteApp session/device capability views now project host audio as an
  explicit unsupported product state (`host_audio_not_implemented`). This
  prevents video readiness from being treated as full audio/video readiness;
  audio capture, audio codec negotiation, and audio E2E remain missing.
- ABI v8 raw stream packets now keep payload bytes out of metadata while
  requiring canonical lifecycle, receipt, terminal, and error fields in the
  metadata contract. This is required for high-frequency RemoteApp/EasyRemote
  media streams, but it does not prove real host audio/video capture or network
  adaptation.
- 2026-08-23 Service owner multihost projection fix:
  the Hub read model now stores Service owner projections per
  `(owner_ura, host_device_ura)` placement while keeping Service out of
  Agent/SystemAgent directory listings. `namespace.resolve` selects a live
  host Device row for Service-owned ability execution. Regression evidence:
  `service_owner_projection_is_fenced_per_host_device`,
  `service_owner_projection_selects_live_host_from_multihost_rows`,
  `service_owner_projection`, and `handle_advertise_abilities`.
- 2026-08-22/23 verification after the Service projection fix:
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
- The cross-device product smoke report now records source/runtime provenance:
  EasyNet-Cli source revision, dirty-state, runtime image name, image id, image
  creation time, and whether `--build` was requested. This is required because
  the default smoke path reuses `EASYNET_RUNTIME_IMAGE`; a stale image failure
  must not be treated as authoritative evidence against the current source.
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
- `tools/scripts/check-remoteapp-main-crate-implementation-tests.sh` now pins
  the correct implementation-test entrypoint. It verifies the standalone
  remote-desktop crate remains a provider shim, then runs main-crate
  implementation tests for app/window target observation, fail-closed
  non-macOS app/window observation, WebRTC media fallback rejection, native
  plugin platform catalogue state, and current-session input policy. This is
  implementation evidence only; it does not prove live product completion.
- RemoteApp session views now expose `input_readiness` as a single
  machine-readable projection for requested mode, effective mode,
  `interactive_ready`, input scope, and blocked reason. This improves frontend
  and E2E diagnosability for the input-injection row, but the row remains
  incomplete until real focus-safe pointer/keyboard injection and latency
  evidence exists.
- RemoteApp session details now separately render input scope and concrete
  pointer/keyboard enablement, for example `input scope display_global ·
  pointer+keyboard` or `input scope display_global · no controls`. This makes
  daemon input authority visible to the user/operator, but it remains
  observability evidence rather than proof of successful OS injection.
- Frontend protocol projection now parses daemon `input_readiness` and input
  sending prefers that runtime readiness over legacy `input_policy`. If the
  daemon reports `interactive_ready=false`, pointer/key frames fail closed
  before transport send. This closes the UI gating seam for requested
  interactive sessions that are correctly downgraded to view-only, while still
  leaving real OS input injection product evidence incomplete.
- Daemon pointer/key input frame parsing now accepts the frontend's
  `sent_at_ms` metadata while keeping strict `deny_unknown_fields` for other
  schema drift. Input applied/rejected events preserve this as
  `client_sent_at_ms`, so the actual data-channel path no longer rejects real
  frontend frames before policy/OS-injection checks and has timestamp evidence
  for later latency E2E.
- Frontend watch_events recovery now consumes daemon input-plane events:
  `INPUT_CHANNEL_OPENED` with blocked activation and `INPUT_FRAME_REJECTED`.
  The UI status shows the daemon reason such as
  `input_injection_unavailable` or `stale_pointer_target_geometry` without
  closing the media/WebRTC transport. This closes a silent-input-failure
  observability seam; it still does not prove successful low-latency OS input
  injection.
- RemoteApp session views now expose a plugin-owned
  `terminal_receipt` projection after explicit `end_session` and lease timeout.
  The receipt binds the session id, target binding epochs, terminal reason, and
  final `SESSION_CLOSED` event id/sequence. This gives frontend and E2E code a
  deterministic session terminal fact instead of inferring closure from the
  last event row. It is not a replacement for canonical Axon Invocation
  receipts.
- RemoteApp consent now separates media/session consent from input-control
  consent. `grant_consent` may mint an explicit `input_control` scoped ticket;
  `create_session` consumes that scope before target binding resolution. Only
  display targets with explicit input-control consent can project
  `display_global` input scope. Window/application targets remain view-only
  because target-scoped keyboard/pointer dispatch still lacks the required
  focus/activation proof. Missing macOS Accessibility permission still reports
  `input_injection_unavailable` in `input_readiness`, so this is a consent and
  policy closure, not successful input-injection E2E evidence.
- `remote_desktop.request_permission` describes and reports both host
  permission axes it handles: Screen Recording for capture and
  Accessibility/input-injection permission for pointer/keyboard control. The
  frontend parses `input_permission` and shows an executable `Request
  permission` recovery action when daemon input readiness reports
  `input_injection_unavailable`. This improves permission correctness and user
  recovery, but still does not prove successful OS input injection.
- `tools/scripts/remoteapp-input-injection-e2e.sh` now provides the input
  injection evidence verifier. It accepts evidence from real host runners and
  requires OS input permission, `input_control` consent, `display_global`
  input scope, focus validation, coordinate mapping validation, positive
  target geometry revision, public RemoteApp session abilities, selected
  Resource URA subject binding, `INPUT_FRAME_APPLIED` pointer/key events with
  `client_sequence` and `client_sent_at_ms`, bounded host-applied latency,
  observed OS pointer/key effects, and visible terminal receipts. Self-test
  validates only the contract; a live host artifact remains required.
- `tools/scripts/remoteapp-media-adaptation-e2e.sh` now provides the media
  adaptation evidence verifier. It accepts evidence from real media runners and
  requires baseline, degraded-network, and backpressure scenarios with
  negotiated video codec, payload content type, transport,
  requested/effective/measured FPS, target and observed bitrate, keyframe
  cadence, bounded frame latency, real host audio, bounded queue depth,
  explicit stale-frame drop policy, bitrate/FPS adaptation or frame-drop
  evidence under impairment, rendered media after adaptation, public RemoteApp
  session abilities, selected Resource URA subject binding, and visible
  terminal receipts. Self-test validates only the contract; a live media
  artifact remains required.
- `tools/scripts/remoteapp-multi-window-tracking-e2e.sh` now provides the
  multi-window tracking evidence verifier. It accepts evidence from real host
  churn runners and requires independent concurrent window streams with
  distinct Resource URAs, session ids, stream ids, media source epochs, and
  frame source ids; non-interleaved frames; ordered move/resize geometry churn
  with increasing target geometry revisions; same-display application
  window-set churn with pending media rebind and `TARGET_REBOUND`; target loss
  with bounded rebind or explicit rebind failure; multi-display application
  pass through `MultiAppSurface` or explicit product unsupported state without
  capture start; public RemoteApp session abilities; selected Resource URA
  subject binding; and visible terminal receipts. Self-test validates only the
  contract; a live tracking artifact remains required.
- The RemoteApp share picker now exposes a non-prompting `Check permissions`
  action before `create_session`. It invokes `remote_desktop.permission_status`
  without a target `subjectURA` and displays Screen Recording plus
  Accessibility/input-injection readiness inside the picker. This closes a
  frontend authorization-preflight seam, while real OS permission and input E2E
  evidence remain required.
- Denied `permission_status` now remains picker-local: it updates visible
  preflight status and offers `Request permission`, but does not set
  `entry.error` or eject the user from the share picker. This preserves the
  intended picker → preflight → request-permission → consent → create flow.
- Frontend RemoteApp creation now sends the same input intent through
  `grant_consent.args.input_control`, `create_session.args.mode`, and
  `create_session.args.input_policy`. The default Interactive path requests
  `input_control=true`; explicit view-only requests carry `input_control=false`
  and disabled keyboard/pointer policy. The CLI frontend boundary gate now
  rejects drift away from this shared-intent contract.
- Pointer input frames now carry the session target tracker
  `target_geometry_revision` when one is available. The daemon rejects pointer
  frames with missing or mismatched target-local geometry revision before
  platform input injection. This closes a stale-transform execution seam; it
  still does not prove successful OS pointer/keyboard injection.
- Frontend session details now render daemon-projected `input_readiness` instead
  of only the user's requested `input_policy`. An interactive request downgraded
  to view-only now appears with the effective mode and blocked reason such as
  `input_injection_unavailable`. This closes an operator-observability seam; it
  does not turn blocked input into successful pointer/keyboard injection.
- Frontend session details now render daemon-projected target recovery state
  from `latestTargetDiagnostic` and `targetTracking`. A lost selected
  window/application target appears as actionable status such as
  `target lost · target_not_found · refresh_targets` instead of a generic
  RemoteApp failure. This closes an application/window observability seam; it
  does not prove real cross-platform capture or multi-window churn E2E.
- The target recovery action is now executable in the frontend action row:
  when daemon `latestTargetDiagnostic.frontendAction` is `refresh_targets`, the
  UI exposes `Refresh targets` and refetches the target inventory through the
  existing `resource.refresh_remote_targets` query path. This still requires the
  user to end/create a new session when the daemon reports
  `new_session_required`; it does not mutate session lifecycle in the browser.
- Frontend session details now render daemon-projected WebRTC route state. A
  host-only route appears as `route host_only · no NAT/relay`, which makes the
  NAT/relay gap visible instead of letting `webrtc ready` imply production
  network readiness. This is route observability evidence, not real
  direct/STUN/TURN/EasyNet relay deployment evidence.
- `tools/scripts/remoteapp-network-fallback-e2e.sh` now provides the network
  fallback evidence verifier. It accepts evidence from a real two-device,
  network-namespace, or deployment runner and requires direct, STUN srflx, TURN
  relay, and EasyNet relay scenarios with connected WebRTC selected
  candidate-pair evidence, rendered media, public RemoteApp session abilities,
  selected Resource URA subject binding, redacted credentials, and visible
  terminal receipts. Self-test validates only the contract; a live network
  artifact remains required.
- Frontend session details now render a compact media quality summary from
  daemon/browser `mediaStats`: bitrate, outbound FPS, aggregate drops, and RTP
  sender backpressure appear as status such as
  `media 18000kbps · 52.5fps · drops 15 · backpressure 3`. This makes adaptive
  bitrate/drop behavior visible to operators; it does not prove real codec
  negotiation, host audio, soak, or degraded-network E2E.
- Frontend protocol/store/UI code now parses and renders daemon-projected
  RemoteApp `terminal_receipt`. After `end_session`, the store retains the
  closed session view with its terminal receipt while clearing `sessionToken`.
  Retained terminal receipts no longer block a later `create_session`; `rdCreate`
  now blocks only non-terminal sessions.
  This gives users and E2E checks a deterministic product terminal fact instead
  of making the closed state vanish as `session=null`; it remains separate from
  canonical Axon Invocation receipts.
- Frontend RemoteApp UI now exposes `Retry session` when daemon/watch-event
  state recommends `retry_session`. The CTA composes existing lifecycle
  abilities in order: `rdEnd`/`remote_desktop.end_session` first, then
  `rdCreate`/`remote_desktop.create_session` for the selected target. Component
  coverage proves this order. This closes the short retry UX seam; it does not
  prove long-outage, crash/restart, revoke, cancel, or timeout E2E.
- Permission revocation now terminates the daemon RemoteApp session with the
  stable `target_permission_revoked` reason and a RemoteApp
  `terminal_receipt`. The frontend closes local transport on the revoked-target
  event, then reads `remote_desktop.show_session` to retain the daemon terminal
  projection and clear the bearer token. This closes the permission-revoke
  lifecycle seam for product state, but still needs real OS permission-revoke
  E2E evidence.
- Frontend device-offline handling now treats presence loss as local transport
  suspension, not session termination. Non-terminal RemoteApp sessions keep
  their daemon session token, resume validates the session with
  `remote_desktop.show_session`, then rebinds WebRTC with transport-failure
  cleanup configured to preserve the daemon session for another reconnect.
  Component and store tests cover the presence drop and rebind path, and the
  CLI frontend boundary gate rejects regressions that clear the session or call
  `rdEnd` from offline presence. This closes the short offline/resume seam; it
  does not prove long-outage, network handoff, process crash, or relay recovery
  E2E.
- Daemon session views now also project target-tracker input loss into
  `input_readiness.blocked_reason=target_input_not_ready`. This keeps the
  public session view aligned with the actual input execution path, which
  already rejects frames when the latest target snapshot has
  `input_enabled=false`.
- The ABI v8 raw-stream metadata contract now reaches release-shape packaging:
  tarball staging, Unix install, sandbox release-install E2E, Windows staging,
  ABI gates, SDK scaffold, and release-package gates all carry
  `include/easynet_cli.exports.v8` beside the base v7 allowlist. This proves
  installed SDK consumers can verify the raw-stream extension contract. It
  does not prove codec negotiation, host audio, relay behavior, or
  cross-device RemoteApp media readiness.
- RemoteApp input sends now have a browser-side RTC data-channel backpressure
  bound and monotonic `client_sequence` telemetry. The remote-desktop plugin
  validates and projects `client_sequence` with applied/rejected input events.
  This closes an input delay/observability seam; it does not prove real
  pointer/keyboard OS injection E2E.
- Diagnostic InvokeBidi input responses now also preserve
  `client_sent_at_ms` and `client_sequence`, including `target_input_not_ready`
  responses after target tracking disables input. This keeps probe evidence
  correlated with frontend frames without adding a second input API.
- Host view-only input safety E2E now opens public
  `remote_desktop.attach` through `easynet ability bidi`, sends pointer/key
  frames with `sent_at_ms` and `client_sequence`, and requires
  `input_scope_unsupported` warnings that echo `client_sent_at_ms` and
  `client_sequence`. This proves the public diagnostic Bidi input path matches
  the app/window view-only policy instead of relying only on session-view
  policy projection.
- Host session timeout E2E now has a runnable entrypoint:
  `host-remoteapp-session-timeout-e2e.sh`. It creates a short-lived session
  through the public CLI, waits past the lease, observes `session_expired`
  through `remote_desktop.show_session`, and invokes
  `remote_desktop.end_session` afterward to prove idempotent terminal receipt
  preservation.
- Host session cancel E2E now has a runnable entrypoint:
  `host-remoteapp-session-cancel-e2e.sh`. It creates a live-target session
  through the public CLI, invokes `remote_desktop.end_session` with
  `user_cancelled`, observes the closed state through
  `remote_desktop.show_session`, and invokes `end_session` again to prove
  idempotent terminal receipt preservation.
- Host permission revoke E2E now has a runnable entrypoint:
  `host-remoteapp-permission-revoke-e2e.sh`. It creates a live-target session
  through the public CLI and waits for real platform permission revoke before
  accepting a public `remote_desktop.show_session` projection with
  `target_permission_revoked`, revoked consent, ordered
  `TARGET_PERMISSION_REVOKED` / `MEDIA_SOURCE_LOST` / `SESSION_CLOSED`
  events, and terminal receipt binding. Its self-test validates the harness
  contract only; product evidence still requires a live run with real platform permission revoke.
- Host session resume E2E now has a runnable entrypoint:
  `host-remoteapp-session-resume-e2e.sh`. It creates a short-lease session
  through the public CLI, invokes public `remote_desktop.refresh_lease`, waits past the original lease, validates the same non-terminal session through
  `remote_desktop.show_session`, and closes it with `resume_e2e_cleanup`.
  This proves daemon/session lease refresh survival; browser/WebRTC rebind,
  long-outage reconnect, crash/restart recovery, and cross-device resume remain
  missing.
- `tools/scripts/remoteapp-crash-restart-recovery-e2e.sh` now provides the
  crash/restart recovery evidence verifier. It accepts evidence from real
  daemon/plugin recovery runners and requires daemon restart of an active
  session, plugin worker restart, terminal receipt replay after crash during
  close, and stale control/invocation socket cleanup. Evidence must prove
  public RemoteApp abilities, selected Resource URA subject binding,
  same-session `show_session` recovery, watch-events/media reattachment,
  recovered WAL/idempotency/replay-guard/lock state, no duplicate invocation
  replay, plugin worker/target-monitor recovery without minting a new public
  session, original terminal receipt replay, endpoint readiness, and visible
  terminal receipts. Self-test validates only the contract; a live recovery
  artifact remains required.

Missing or insufficient product evidence:

- Cross-platform capture implementation/evidence using
  `remoteapp-cross-platform-capture-e2e.sh`: macOS display/window/application
  live pass plus Windows/Linux capture or explicit product unsupported state.
- Real input injection E2E for pointer/keyboard using
  `remoteapp-input-injection-e2e.sh` with a live artifact.
  Current source/product-path progress: frontend input sending now rejects
  missing or stale pointer `target_geometry_revision` before WebRTC
  data-channel send, while daemon stale-revision rejection remains the
  authoritative execution boundary.
- Audio/video media adaptation E2E using
  `remoteapp-media-adaptation-e2e.sh` with a live artifact proving negotiated
  codec, host audio, FPS, bitrate, bounded queues, backpressure, drop policy,
  adaptation under degraded network, rendered media after adaptation, and
  terminal receipts.
  Current source/product-path progress: frontend protocol projection now parses
  daemon `audio` and `production_readiness.audio_*` fields, and session details
  show `audio blocked · host_audio_not_implemented`. This is product
  transparency, not host-audio implementation evidence.
- Multi-window tracking E2E using
  `remoteapp-multi-window-tracking-e2e.sh` with a live artifact proving
  independent concurrent window streams, non-interleaved frames, move/resize
  geometry revisions, same-display application window-set rebind, target loss
  rebind/failure behavior, multi-display application pass or explicit product
  unsupported state, and terminal receipts.
- Crash/restart recovery E2E using
  `remoteapp-crash-restart-recovery-e2e.sh` with a live artifact proving
  daemon/plugin restart recovery, same-session `show_session`, watch/media
  reattachment, recovered WAL/idempotency/replay-guard/lock state, original
  terminal receipt replay, stale socket cleanup, endpoint readiness, and
  terminal receipts.
- Session resume/reconnect/revoke/crash-restart recovery E2E.
- Real direct/STUN/TURN/EasyNet relay reachability matrix using
  `remoteapp-network-fallback-e2e.sh` with a live artifact.
  Current source/product-path progress: daemon transport views now project
  browser `client_ice_servers`, and the frontend WebRTC path consumes that
  session-projected config instead of hard-coding an empty ICE server list.
  This is required plumbing; it is not real relay reachability evidence.
- Frontend full lifecycle E2E across Browser/Tauri surfaces, using
  `frontend-remoteapp-browser-lifecycle-e2e.sh` with a live artifact.
- RemoteApp-specific cross-device smoke/regression with remote target
  inventory, real display/window/application capture, input policy, and
  teardown.

# Evidence Audit — RemoteApp Product Closure

Authoritative product readiness source:

- `docs/design/remoteapp-product-readiness-audit-2026-08-22.md`

Current conclusion:

- Targeted-session architecture: implemented with source and host-E2E harnesses.
- Full interactive RemoteApp product: incomplete.
- 2026-08-27 live Linux/X11 application execution selected two exact windows,
  rejected a black-gap centre probe, then passed pointer/key input when the
  Browser chose a committed surface. Relay-only WebRTC, watch, rendered media,
  no scope widening, and terminal cleanup were observed; formal verification
  required removal of a macOS-only discovery-scope assumption.
- RemoteApp implementation test evidence must come from the main EasyNet crate,
  not the standalone `easynet-plugin-remote-desktop` package. The standalone
  package is a provider/export shim whose zero-test result does not exercise
  the daemon-embedded implementation.
- 2026-08-28 receiver readiness no longer treats Browser `presenting` as
  decoded media proof. Product readiness requires a fresh, daemon-admitted
  render tuple bound to the exact session, transport epoch, target binding,
  media-source epoch, pipeline, codec and dimensions. The authored descriptor
  and NativeStatic runtime schema are parity-tested.
- 2026-08-28 host-audio offer readiness comes from a plugin-owned runtime
  probe coordinator. Its wake queue has capacity one, refresh storms coalesce
  into one bit, and invalidation is synchronous and source-scoped. Native
  PipeWire/WASAPI discovery now runs in the independent one-shot
  `easynet-remoteapp-media-host` capability mode: a result is committed only after exact
  schema validation and successful child exit, while timeout/protocol failure
  kills and reaps the process. This closes capability-probe thread leakage; it
  does not isolate active PCM capture.
- 2026-08-28 target inventory and per-input target guards no longer execute
  native enumeration in an unkillable daemon thread. The plugin-private
  `easynet-remoteapp-native-host` executable now serves two independently
  supervised bounded lanes over a strict 4 MiB length-prefixed protocol.
  Deadline, protocol, stale-generation, disconnect, and shutdown paths kill and
  wait for the child before reuse. Unsolicited extra responses use non-blocking
  bounded delivery and retire the generation instead of deadlocking its reader.
  Local macOS real-process, injected-hang, and extra-response tests pass. Media
  capture/encode/PCM streaming have not moved behind this boundary,
  and Windows/Linux signed release execution remains unproved.
- The target-observation helper no longer depends on the root `easynet` crate.
  Its private protocol crate owns DTOs, validation, and framing; the native-host
  library owns OS enumeration. A Cargo dependency-graph gate rejects Runtime,
  Axon, and tonic reachability from the helper. This is target-observation
  isolation only, not media data-plane isolation.
- The media-host executable likewise has no Runtime/Axon/tonic dependency.
  Its protocol carries only compiled/runtime/source readiness and bounded
  diagnostics; daemon-owned generation, TTL, invalidation and admission state
  do not cross the boundary. Real-process round-trip and injected-hang recovery
  are covered locally, while signed Windows/Linux execution is still open.
- Focused verification passes Rust compile, host-audio capability,
  render-evidence, transport-state and product-readiness tests; frontend
  typecheck and 105 focused tests; and the performance/frontend boundary gates
  with their mutation suites. Live cross-platform and same-campaign product
  evidence remain open.

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
- `remoteapp-product-completion-e2e.sh`

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
  `DeviceMediaAccess` UI flow, host permission-subject preflight, separate
  window/application target picker freshness, decoded-frame WebRTC, and
  view-only input safety. An explicit --run report remains required
  before treating it as environment evidence; the default skipped
  report only proves the harness contract exists.
- `tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh` now provides the
  Browser/Tauri lifecycle evidence verifier. It accepts evidence from a real UI
  runner and requires `real_browser_tauri_lifecycle`, `component_mock=false`,
  `real_backend_runtime=true`, ordered picker/permission/consent/create/attach/
  watch/media/media-pipeline-support/input/end/terminal-receipt steps, public
  RemoteApp ability names, host-local `permission_status`, selected Resource
  URA subject binding for session abilities, visible `media_pipeline_support`,
  and no product-complete claim. Self-test validates only the contract; a live
  Browser/Tauri artifact remains required.
- `tools/scripts/remoteapp-product-completion-e2e.sh` now provides the single
  aggregate product-completion evidence gate. A passing aggregation emits only
  an eligible candidate with `product_complete_claim=false`; it cannot mint a
  final product claim. It requires passed report JSONs
  from frontend product-flow, Browser/Tauri lifecycle, cross-device smoke,
  cross-platform capture, input injection, media adaptation, multi-window
  tracking, network fallback, window/application session timeout,
  window/application session cancel, window/application permission revoke,
  window/application session resume, and crash/restart recovery. It rejects
  missing reports, child verifier `product_complete_claim=true`, and
  cross-device reports where `local_provider_boundary_only=true`. It also
  verifies the stable `script` identity for each required report, including the
  host timeout/cancel/revoke/resume lifecycle reports, and pins those lifecycle
  reports to the exact expected `target_kind` so one target kind cannot satisfy
  the other. It now also requires existing `evidence_json` artifacts for the
  domain verifier reports, requires cross-platform capture and input-injection
  platform summaries to be `passed` for macOS, Windows, and Linux instead of
  explicit `unsupported`, and requires explicit passed
  frontend product-flow steps for Browser/Tauri, cross-device,
  permission-subject, separate window/application target-picker freshness,
  window/application decoded-frame, and
  window/application view-only-input coverage with `target_kind=both`, and
  traceable `result.json` step artifacts plus subreport/evidence artifacts for
  Browser/Tauri, cross-device, and host product-flow steps. Host product-flow
  verifier reports now expose stable `script` identity for permission-subject,
  both target-picker variants, decoded-frame, and view-only-input evidence;
  target-picker, decoded-frame, and view-only-input subreports must also match
  the exact `target_kind` required
  by their frontend step, so window evidence and application evidence cannot be
  swapped. For every required report or product-flow subreport that names a
  live `evidence_json` artifact, the aggregate gate now parses that artifact
  and requires its own `status=passed`; this prevents a passed summary report
  from pointing at empty, failed, or invalid evidence. It also requires
  non-empty observed caller/provider device pairs with distinct device URAs in
  the cross-device report. This gate is an aggregate completion guard, not a
  substitute for any live domain artifact, and it rejects target-narrowed,
  target-swapped, failed-evidence, or empty-shell frontend product-flow
  evidence.
- `tools/scripts/remoteapp-product-finalize.py` is the only boundary that may
  emit `product_complete_claim=true`. It verifies a dedicated independent
  product-completion authority signature over the exact candidate and signed
  campaign/source/build tuple before atomically consuming the campaign replay
  id. Replay records bind the completion-statement and final-report digests, so
  an exact crash-recovery retry is idempotent while an alternate decision fails
  closed. Production trust and replay paths are fixed system authority and are
  not caller-selectable. The final report includes the exact candidate bytes;
  the same tool exposes standalone signature, canonical-projection, and replay
  verification instead of asking consumers to trust a mutable claim label.
  Its `prepare` command independently verifies the complete 19-domain matrix
  and emits canonical DSSE PAE bytes for an external KMS/HSM. `assemble`
  accepts only a valid 64-byte Ed25519 signature from a completion-role key;
  private signing keys are never accepted by the tool.
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
- 2026-08-23 Hub API readiness evidence:
  `target/e2e/hub-api-readiness/20260823-rich-failure-check-70909/report.md`
  fails before RemoteApp product-flow execution with
  `runtime_status=projection_present_process_missing`,
  `connection_state=START_FAILED_CREDENTIAL_VERIFY`,
  `connection_failure.stage=T06_VERIFY_CREDENTIAL`,
  `hub_endpoint=https://127.0.0.1:50443`, and `hub_api_endpoint=null`. This is
  current environment evidence against product completion, not a RemoteApp
  frontend/capture/media/input pass.
- 2026-08-23 full product-flow attempt:
  `target/e2e/frontend-remoteapp-product-flow/20260823-live-preflight-82429/report.md`
  fails at `hub-api-readiness-preflight` and propagates the same
  credential-verification diagnostics into the product-flow report. No
  frontend, host capture, media, or input product evidence ran after that
  upstream gate failed.
- 2026-08-23 hydrated runtime-status follow-up:
  `target/e2e/hub-api-readiness/20260823-hydrated-health-report-21626/report.md`
  and
  `target/e2e/frontend-remoteapp-product-flow/20260823-hydrated-health-report-21627/report.md`
  resolve `hub_api_endpoint=http://localhost:8080` from current credentials and
  then fail on the real Hub API health probe:
  `http://localhost:8080/api/v1/health` returns connection refused. Docker is
  reachable, but product-flow still stops before frontend, host capture, media,
  or input evidence.
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
- Current 2026-08-23 local product-flow evidence:
  `target/e2e/frontend-remoteapp-product-flow/20260823-both-current-69931/report.md`
  passed all bounded local steps on current HEAD with `target_kind=both`,
  including decoded-frame WebRTC and view-only input safety for both window and
  application targets. This strengthens local macOS frontend + daemon + host
  evidence but does not prove cross-platform, cross-device, host-audio,
  real-input-injection, NAT/relay, or Browser/Tauri product completion.
- Current-checkout target-picker freshness evidence on 2026-08-26 passed
  separately for window and application targets:
  `target/e2e/host-remoteapp-target-picker-freshness/20260826-073415-window-live/report.json`
  binds `window_id + owner_pid`; and
  `target/e2e/host-remoteapp-target-picker-freshness/20260826-073402-application-live/report.json`
  binds the stable application identity, owner pid, two native window ids,
  front-to-back surface membership, and `window_set_epoch` while proving the
  target is process-scoped. These artifacts close E2E-01 picker selection for
  both supported target kinds only; they do not prove capture or a session.
- The AppKit fixture now stores LaunchServices executables and command/ack IPC
  in a fixture-owned physical temporary directory, while reports remain in the
  requested output directory. Cleanup validates both exact runtime path and
  process command before termination/removal. This removes the macOS
  `~/Documents` privacy seam that previously allowed application windows to
  launch but prevented focus/control acknowledgement.
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
  plugin platform catalogue state, production-vs-diagnostic target-subject
  projection, and current-session input policy. This is implementation evidence
  only; it does not prove live product completion.
- RemoteApp device capability projection now separates
  `metadata.production_target_subjects` from
  `metadata.diagnostic_target_subjects`. Unavailable or permission-blocked
  production backends no longer project app/window/application as current
  production subjects, while the xcap diagnostic fallback stays display-only.
  The projection uses the same runtime native backend descriptor as
  `production_gate_view`, so macOS permission denial and non-macOS
  `not_installed` state are represented consistently.
- `check-remoteapp-lifecycle-input-boundary.sh` and its mutation test fixture
  now pin that production-vs-diagnostic target-subject projection. Removing the
  runtime-native descriptor, production-ready gate, display-only diagnostic
  subjects, blocked reason, or production subject source fails the boundary.
- RemoteApp device capability projection now exposes
  `metadata.platform_support` for macOS, Linux, and Windows. macOS target rows
  follow the native production gate, Linux display is diagnostic-only, Linux
  window/application are unsupported, and Windows display/window/application are
  unsupported until native backends exist. This is explicit unsupported product
  state, not cross-platform capture completion.
- RemoteApp device capability projection now exposes
  `metadata.input_control_support` for macOS, Linux, and Windows. macOS display
  input follows runtime Accessibility/input-injection permission, macOS
  window/application input remains unsupported until target-scoped dispatch is
  safe, and Linux/Windows input injection is unsupported until native backends
  exist. This is explicit unsupported product state, not successful OS input
  injection evidence.
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
- `tools/scripts/host-remoteapp-turn-relay-e2e.sh` now closes the focused TURN
  child with a reproducible coturn fixture. The 2026-08-26 live run constrained
  the real Browser to relay-only ICE, observed three server-side allocations,
  selected a connected/nominated/succeeded relay route with positive
  bidirectional bytes, presented three later frames, retained the canonical
  User/SystemAgent/Device/Resource identity split, emitted a `caller_ended`
  receipt, and restored the ordinary daemon. Direct and Hub-owned EasyNet relay
  children have also passed; STUN srflx remains required for the aggregate
  network matrix.
- `tools/scripts/host-remoteapp-direct-e2e.sh` now makes the direct child
  reproducible without treating a coincidental host candidate as proof. It
  removes daemon STUN/TURN/EasyNet relay configuration, requires zero projected
  ICE URLs and host-only local/remote SDP, validates the selected direct pair,
  later media and terminal cleanup, then restores the ordinary daemon. The
  2026-08-26 live macOS/window run passed with a connected/nominated/succeeded
  UDP host/host pair, positive bidirectional bytes, three later frames, and a
  `caller_ended` receipt. STUN srflx remains open.
- The 2026-08-26 native decoded-frame rerun closed the macOS single-window and
  single-application TURN children without conflating host audio capability
  with negotiated media scope. The window move/resize proof at
  `/tmp/remoteapp-window-move-resize-turn-live-20260826-v3/report.md` and the
  application proof at `/tmp/remoteapp-application-turn-live-20260826-v5/report.md`
  both report `media_scope=video_only`, `audio_required=false`, real coturn
  relay selection, decoded selected sentinel pixels, zero unrelated sentinel
  pixels, and `production_readiness.ready=true`. The application fixture now
  uses independent LaunchServices-started `.app` bundles; its binding keeps
  `display_id=null` and canonical `display_ids=[1]` instead of inventing a
  display routing identity. The durable shape now uses recovery schema v2;
  schema-v1 rows migrate only when the missing topology is derivable from a
  positive committed `display_id`, while ambiguous process-scoped rows fail
  closed. Recovery tests and the full RemoteApp Rust suite pass (492/492).
  These children do not close the multi-window,
  cross-display, real input, or cross-platform matrices.
- `tools/scripts/host-remoteapp-stun-srflx-e2e.sh` now supplies the focused
  STUN-only child contract without treating any reflexive-looking RTCStats row
  as proof. It rejects the non-routable macOS Docker Desktop topology, runs an
  address-redacted RFC 5389 Binding observer at the provider boundary, requires
  an externally reachable VM-NAT Browser context, constrains Browser outbound
  trickle and embedded offer SDP to `srflx`/`prflx` while retaining provider inbound
  `host`/`srflx`/`prflx`, counts accepted and rejected candidates,
  requires the selected Browser-local candidate to be reflexive and the
  projected local SDP to contain no host candidate, requires a server-observed
  binding in the correct time interval, bounds the Browser child, validates
  later media/terminal evidence, and restores the ordinary daemon. The observer
  passed an independent coturn client probe and returned a real VM-NAT
  reflexive mapping. Focused positive and mutation gates pass, but the temporary
  VM context was removed after topology proof and the exact active daemon still
  lacks Screen Recording permission. No complete Browser child exists, so the
  STUN route remains open.
- Frontend session details now render a compact media quality summary from
  daemon/browser `mediaStats`: bitrate, outbound FPS, aggregate drops, and RTP
  sender backpressure appear as status such as
  `media 18000kbps · 52.5fps · route direct · drops 15 · backpressure 3`. This makes adaptive
  bitrate/drop behavior visible to operators; it does not prove real codec
  negotiation, host audio, soak, or degraded-network E2E.
- Frontend protocol/UI now parses and renders daemon-projected
  `media_pipeline_support`: video-only scope, H.264 pipeline identity,
  bounded stale-frame drop policy, and product blockers such as
  `host_audio_not_implemented` appear in session details. This keeps frontend
  product state aligned to daemon capability projection; it does not prove
  host-audio or degraded-network E2E.
- Frontend protocol/store/UI code now parses and renders daemon-projected
  RemoteApp `terminal_receipt`. After `end_session`, the store retains the
  closed session view with its terminal receipt while clearing `sessionToken`.
  Retained terminal receipts no longer block a later `create_session`; `rdCreate`
  now blocks only non-terminal sessions.
  This gives users and E2E checks a deterministic product terminal fact instead
  of making the closed state vanish as `session=null`; it remains separate from
  canonical Axon Invocation receipts.
- Frontend RemoteApp UI now exposes `Retry session` when daemon/watch-event
  state recommends `retry_session`. The CTA invokes the store-owned same-session
  transport recovery path: validate with `show_session`, retire the old local
  PeerConnection, renegotiate with `set_description`, attach a strictly newer
  transport epoch, restart `watch_events`, and refresh the existing lease.
  Component/store coverage proves the retry does not call `end_session` or
  `create_session`. This closes the semantic retry UX seam; it does not prove
  long-outage, crash/restart, revoke, cancel, or timeout E2E.
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
- Browser transport resume is now an independent product-gate domain rather
  than an alias for lease refresh. The Browser lifecycle runner accepts paired
  external disconnect/reconnect commands and requires the same public session,
  retirement of the old PeerConnection, a strictly newer daemon-issued
  transport epoch, a newly connected PeerConnection, `watch_events`
  reattachment, a decoded frame after resume, and preserved input authority.
  The aggregate product gate rejects reports without this summary and its
  self-test proves that reusing the old PeerConnection is rejected. A
  2026-08-26 live Browser run now passed this contract across a real paired
  daemon stop/start: the same window session survived, the old PeerConnection
  closed, transport epoch increased from `1787686710123091` to
  `1787686896984117`, a new PeerConnection and `watch_events` stream connected,
  and a `1688x1080` frame rendered after resume. Input authority was preserved
  but macOS Accessibility remained policy-blocked. This proves orderly daemon
  restart recovery, not `kill -9`, plugin-worker-only failure, or
  crash-during-close receipt replay.
- Latest bounded local lifecycle live evidence on 2026-08-23 passed for both
  window and application targets using catalog-resolved full Ability URAs and
  the session approval receipt as scalar causal context:
  `target/e2e/host-remoteapp-session-timeout/20260823-live-window-causal-222646-11519/report.md`,
  `target/e2e/host-remoteapp-session-cancel/20260823-live-window-causal-222700-12564/report.md`,
  `target/e2e/host-remoteapp-session-resume/20260823-live-window-stable-222830-19233/report.md`,
  `target/e2e/host-remoteapp-session-timeout/20260823-live-application-causal-222846-20408/report.md`,
  `target/e2e/host-remoteapp-session-cancel/20260823-live-application-causal-222859-21255/report.md`,
  and
  `target/e2e/host-remoteapp-session-resume/20260823-live-application-stable-222859-21261/report.md`.
  This is local daemon lifecycle evidence only; permission revoke,
  long-outage reconnect, browser/WebRTC rebind, crash/restart recovery,
  cross-device transport, and cross-platform OS behavior remain open.
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
- `tools/scripts/host-remoteapp-target-monitor-worker-recovery-e2e.sh` now
  closes the target-monitor worker-only child scenario with live macOS/window
  evidence rather than a component fixture. The 2026-08-26 v4 campaign kept
  exact feature daemon PID `62280` J800, preserved Browser session
  `rdp-66843994a4396d038cc76b94` and all consent/binding/transport/media epochs,
  bound public and durable ordered worker events from failed generation `1` to
  replacement generation `2`, rendered a later frame, and ended the same
  session with a visible `caller_ended` receipt. The report is
  `/tmp/remoteapp-target-monitor-worker-live-20260826-v4/report.md`. This does
  not close Windows named-pipe, cross-device, or aggregate recovery evidence.
- Latest live crash/restart probe:
  `target/e2e/remoteapp-crash-restart-probe/20260822-223509-45956`.
  The probe killed the daemon with active RemoteApp session
  `rd-crash-probe-45956`, restarted it, and public
  `remote_desktop.show_session` returned `session_not_found`. This remains the
  latest live and therefore authoritative historical negative artifact, but it
  predates the current startup-rehydration wiring and must be rerun before it
  can describe the current executable behavior.
- `RemoteDesktopRecoveryStore` is now wired into plugin startup. Non-terminal
  rows rehydrate as degraded sessions, retain the session token, consent,
  target binding, input blocker, bounded event replay, and transport epoch high
  watermark, then restart lease and target monitoring. Unit/runtime tests prove
  public `show_session`, `watch_events`, `end_session`, and a newer media epoch
  against the recovered aggregate. This is Stage 1 source evidence; it does not
  satisfy the live crash/restart verifier.
- Recovery persistence now enforces a 4 MiB per-snapshot bound while reading
  before JSON decode and while serializing before atomic write. Commit, load,
  and delete share one store-global process lock. Session pruning returns the
  exact removed terminal ids, excludes those rows from subsequent persistence,
  and deletes their durable snapshots; regression coverage proves a successor
  session cannot leave the pruned tombstone on disk. Startup derives its row
  ceiling from active capacity plus the canonical four-terminal-per-active
  policy (640 rows for the default 128 sessions), caps all directory entries
  including sidecars, and rejects aggregate snapshot bodies above 64 MiB before
  JSON decode. Tests cover row, directory-entry, and byte storms.
- The frontend Retry session action previously translated daemon
  `retry_session` into `end_session` followed by `create_session`, contradicting
  the daemon recovery contract and minting a new session/consent path. It now
  invokes the shared store-owned transport retry state machine: public
  `show_session`, a new PeerConnection, `set_description` with a strictly newer
  transport epoch, `watch_events` reattachment, and lease refresh all preserve
  the original session id/token/consent. Source tests reject end/create during
  retry. Additional regressions suspend the device while `show_session` and
  `set_description` are independently in flight; a monotonic retry generation
  fences both stale continuations, leaves the original session preserved, and
  prevents a closed PeerConnection from being resurrected. A live
  daemon-crash/browser-reconnect artifact remains required.

Missing or insufficient product evidence:

- 2026-08-28 red-team review found that Linux X11/XTest cannot isolate a whole
  press-to-release lifecycle to one Window/Application target. A focus switch
  between down and up could otherwise release into another application. The
  resolver now models this explicitly: Linux display input remains
  `display_global`, while Linux Window/Application sessions remain `view_only`
  even after consent. `target_local` must not be restored until a target-bound
  input device/session exists. The target-observation executor itself is now a
  killable deadline-bounded helper process; that does not make XTest target
  scoped.
  Focus tests (4), target-local policy/guard tests (4), media-scope readiness
  tests (2), the lifecycle/input checker, and its full mutation suite pass.
  The Windows cross-target attempt remains environment-blocked in `ring` because
  this macOS host has no `x86_64-w64-mingw32-gcc`; it is not pass evidence.
  Real Windows/Linux pointer/key effects and Linux Wayland portal support remain
  release-blocking.
- Permission projection now reflects execution reality instead of a generic
  macOS-shaped contract. macOS reports Accessibility, Windows reports User32
  SendInput, and Linux reports X11/XTest. Linux Wayland and a daemon without an
  X11 `DISPLAY` fail closed with typed unavailable reasons, and a permission
  request is marked attempted only when the host can actually prompt. Unit and
  frontend projection tests cover the contract; real Windows/Linux host
  permission and OS-effect evidence is still required.
- Target monitor workers now retain only session, transport, and recovery
  components. They never retain the aggregate `RemoteDesktopPlugin`, which
  removes the plugin-drop/supervisor/generation circular join observed during
  the full RemoteApp test suite. The regression suite proves acyclic ownership;
  live plugin crash/restart recovery remains a separate product requirement.
- Verification on 2026-08-24:
  - `cargo test --lib daemon::plugins::remote_desktop:: -- --nocapture` passed
    394 tests with no supervisor self-join/resource-deadlock failure.
  - `npm test -- --run src/store/media-channel-store.test.ts` passed 21 tests,
    and `npm run build` passed in `EasyNet/Frontend`.
  - `bash tools/scripts/check-remoteapp-product-closure-audit.sh` passed.
  - Linux and Windows cross-target `cargo check` attempts stopped in `ring`
    before project compilation because this host lacks
    `aarch64-linux-gnu-gcc` and `x86_64-w64-mingw32-gcc`; these attempts are
    environment/toolchain blockers and are not cross-platform pass evidence.

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
  Current source/product-path progress: the macOS ScreenCaptureKit path captures
  host audio, encodes bounded 20 ms Opus packets, and sends them on the shared
  WebRTC peer connection. Audio capture and encoded-packet queues are now hard
  bounded, stale packets are dropped, and the only RTP writer runs in a
  session-owned abortable task so slow audio transport cannot block video
  adaptation, target rebind, cancellation, or terminal progress. Runtime stats,
  the evidence verifier, and frontend details project the queue bound, observed
  depth, drop policy, isolated-writer state, and drop/error counters. This is
  executable source and contract evidence, not a live second-device host-audio
  or degraded-network artifact.
  Verification on 2026-08-24 passed the focused native-media tests (5/5), the
  full RemoteApp module suite (396/396), media-adaptation verifier self-test,
  product-closure audit, focused frontend protocol/panel tests (16/16), and the
  frontend production build. A subsequent live-environment probe could not
  start the real matrix: Hub `:8080` and frontend `:3000` were unreachable,
  the daemon reported `DEGRADED/T09_OPEN_SELF_SESSION` with no admitted Hub
  session, and `docker ps` did not return before it was interrupted. Therefore
  no baseline/degraded-network/backpressure product artifact was produced.
- Multi-window tracking E2E using
  `remoteapp-multi-window-tracking-e2e.sh` with a live artifact proving
  independent concurrent window streams, non-interleaved frames, move/resize
  geometry revisions, same-display application window-set rebind, target loss
  rebind/failure behavior, multi-display application pass or explicit product
  unsupported state, and terminal receipts.
  Current live progress on 2026-08-27: Linux/X11 application window-set churn
  passed through the real Browser, Hub, paired provider, and relay-only WebRTC
  path. One selected application session closed and recreated its secondary
  native window, advanced binding/identity/geometry epochs, exposed the new
  two-XID set through `show_session`, rendered another decoded frame, applied a
  second pointer/key sequence, preserved `scope_widened=false` and
  `display_fallback_used=false`, and ended with a visible terminal receipt.
  Evidence:
  `/tmp/remoteapp-linux-provider-probe.YkH7U3/browser-application-evidence-v40-window-set-churn.json`.
  Formal Browser lifecycle verifier passed. This closes one application churn
  scenario only; independent concurrent streams, move/resize event taxonomy,
  target-loss deadline behavior, multi-display policy, macOS, and Windows
  remain required by the aggregate multi-window gate.
  A separate Linux/X11 focus-recovery live run also passed: an unrelated native
  process displaced focus, the target monitor projected `target_blurred`, the
  first Browser pointer intent invoked `remote_desktop.focus_target`, the
  committed focus epoch advanced from 3 to 4, and pointer/key input was then
  applied on the relay session. Evidence:
  `/tmp/remoteapp-linux-provider-probe.YkH7U3/browser-application-evidence-v41-focus-recovery.json`.
  The formal Browser verifier passed; this is not macOS/Windows focus evidence.
  The Browser lifecycle product item now consumes a dedicated two-leaf matrix
  instead of requiring one impossible single-target run to claim both kinds.
  A real Linux window focus-recovery leaf and the application focus-recovery
  leaf aggregated successfully at
  `/tmp/remoteapp-linux-provider-probe.YkH7U3/v42-v41-browser-target-matrix/report.json`.
  The product-completion audit accepted `browser_lifecycle` with zero item
  errors while keeping the overall result failed with 19 missing/campaign
  requirements. This is the intended fail-closed state.
  A second live Linux/X11 application run then closed the move/resize taxonomy
  seam. It kept the same two native XIDs and target-identity epoch, advanced
  binding epoch 1→2 and geometry revision 1→2 only after the rebuilt media
  source committed, emitted ordered `TARGET_MOVED` then `TARGET_RESIZED`,
  rendered nine further frames, and applied another pointer/key sequence against
  the new 840x500 committed application bounds. The selected route was
  relay-only and neither scope widening nor display fallback occurred. Evidence:
  `/tmp/remoteapp-linux-provider-probe.YkH7U3/browser-application-evidence-v46-geometry-events.json`.
  Formal verifier report:
  `/tmp/remoteapp-linux-provider-probe.YkH7U3/v46-geometry-formal-verifier/report.json`.
  This closes the Linux application geometry-churn leaf; independent concurrent
  sessions, target-loss deadlines, multi-display policy, and macOS/Windows
  evidence remain open.
- Crash/restart recovery E2E using
  `remoteapp-crash-restart-recovery-e2e.sh` with a live artifact proving
  daemon/plugin restart recovery, same-session `show_session`, watch/media
  reattachment, recovered WAL/idempotency/replay-guard/lock state, original
  terminal receipt replay, stale socket cleanup, endpoint readiness, and
  terminal receipts.
- Session resume/reconnect/revoke/crash-restart recovery E2E.
- Aggregate direct/STUN/TURN/EasyNet relay reachability matrix using
  `remoteapp-network-fallback-e2e.sh` with a live artifact.
  The reproducible direct, TURN, and Hub-owned EasyNet relay children pass; the
  constrained STUN harness is implemented but has no passing live child. No
  single signed campaign yet aggregates all four routes.
  Hub relay refresh and Browser transport resume now also have one composable
  live gate instead of two unrelated source claims. The EasyNet relay runner's
  `--refresh-resume` mode accelerates the real Hub TTL, waits across the
  daemon-owned refresh threshold, restarts the paired daemon, and requires the
  Browser to observe a distinct redacted lease while preserving the public
  session and binding a newer WebRTC transport. A dedicated fail-closed
  verifier binds both lease IDs, session, Resource, transport epochs,
  reattached `watch_events`, post-resume media, terminal receipt, and redaction
  into the canonical network scenario. Skip/self-test reports expose zero live
  coverage. The harness integration tests pass; the live
  `--run --refresh-resume` artifact remains required.
- Frontend full lifecycle E2E across Browser/Tauri surfaces, using
  `frontend-remoteapp-browser-lifecycle-e2e.sh` with a live artifact proving
  visible media pipeline support in addition to picker/permission/consent,
  media, input/control, end, and terminal-receipt steps.
- RemoteApp-specific cross-device smoke/regression with remote target
  inventory, real display/window/application capture, input policy, and
  teardown.

## 2026-08-28 — Linux exact host-effect evidence hardening

- Window v63 preserved raw target-process observations instead of grafting
  daemon event ids into the observer log. It proved exact XID input before and
  after resize, stable observer identity, normal guarded release, and no input
  on the second visible window.
- Application v66 selected the committed two-XID set owned by PID `5110`,
  recovered target focus from epoch `2` to `3`, and applied pointer/key down/up
  with strictly increasing Runtime and client sequences. An independent PID
  `5314` kept two visible windows and observed zero input events.
- The formal Browser lifecycle verifier passed v66 with
  `focus_recovery_verified=true`, `host_input_effects_verified=true`, and
  `input_interaction_sequence_verified=true`.
- This evidence remains one Linux/X11 leaf. The run itself reports missing media
  adaptation evidence, non-negotiated production codec readiness, and
  unavailable host audio. It cannot support a product-complete claim.
- Three subsequent independent Application sessions, v70/v71/v72, passed the
  real Browser runner and the formal lifecycle verifier with distinct session
  ids. Each run displaced focus, committed a newer focus authority, applied
  normal guarded pointer/key down/up frames to the selected two-XID
  application, observed zero input in the independent PID, and completed
  terminal cleanup. Evidence and reports are under
  `/tmp/remoteapp-linux-provider-probe.YkH7U3/browser-application-evidence-v7{0,1,2}-v17-stability.json`
  and the corresponding `browser-application-v7{0,1,2}-v17-stability-verifier/`
  directories. All three retain `product_complete_claim=false`.

## 2026-08-28 — Shared-lane dimension-convergence verification

- `cargo test -p easynet-remoteapp-native-protocol shared_media_lane --
  --nocapture` passed seven slot/state/lease tests, including the exact
  `Bytes` owner lifetime and fixed-notification-to-WebRTC borrowed-view path.
- The comparative 128×256 KiB fixture passed. Payload-pipe v1 recorded 1,280
  allocation calls and 33,612,544 allocated bytes; shared-lane v2 recorded 128
  owner allocations and 13,312 allocated bytes. This is same-process hot-path
  evidence, not a cross-device zero-copy claim.
- `cargo test --features axon-pb --lib
  daemon::plugins::remote_desktop::transport::webrtc_hosted_media::tests --
  --nocapture` passed all seven hosted-media tests. The new regression proves
  that a committed 200×100 target remains 200×100 under a 1280×720
  `scale_mode=native` upper bound.
- A current native Linux artifact bundle was built at
  `../EasyNet/target/dev-backend/cli-artifacts-v4` and verified by the artifact
  bundle gate. The device/provider image was rebuilt from that exact manifest.
- The first Browser rerun failed before media admission because
  `remote_desktop.permission_status` exceeded the 90-second Browser deadline
  while the Hub was serving several concurrent provider/catalog requests.
- The immediate retry passed permission discovery but `create_session` rejected
  the selected live inventory row as `target_stale`; the Hub delay outlived the
  inventory freshness window. Evidence is under
  `target/e2e/remoteapp-linux-provider-browser/native-dim-live/browser{,-retry}`.
- These are correctly failed live artifacts. They do not prove sustained frame
  delivery after the dimension fix, and product completion remains false.

## 2026-08-28 — Real Browser sender-service failure and corrected diagnosis

- The next live Linux/X11 window run reached the real Browser, selected an exact
  480×320 native window, negotiated WebRTC, and rendered one 480×320 frame.
  Evidence:
  `target/e2e/remoteapp-linux-provider-browser/manual-schema3-window-recovery-fix/evidence.json`.
- The run then stalled. The device was configured for 14,000 kbit/s but emitted
  only about 132.5 kbit/s of encoded media. Its one-frame daemon queue dropped
  the subsequent dependency chains and the Browser presented only one frame.
  The only RTP-write latency sample was about 73 ms, slower than the configured
  30 fps service interval. This contradicts sustained-media acceptance even
  though the first-frame checkpoint passed.
- The Browser also reported `availableOutgoingBitrate=300000` on its selected
  candidate pair, but that field describes the Browser's outgoing/upload path,
  not capacity for device-to-Browser video. Its `report_client_state` Ability
  took about 12.8 seconds to complete, so it is directionally and temporally
  invalid as an encoder control input. It remains diagnostic evidence only.
- The device-local sender control loop is now implemented in source. Each
  transport generation retains its negotiated video `RtpSender`, consumes only
  fresh `remote-inbound-rtp` RTCP Receiver Reports, and combines that loss/RTT
  pressure with measured RTP-writer p95 service time and bounded queue pressure.
  Hosted and baseline paths share the same FPS policy; a 73 ms p95 sample maps
  30 fps to 10 fps with 25% service headroom. Browser decode/freeze reports
  remain useful for audit/readiness but are not the primary congestion loop.
- The shared-lane microbenchmark remains valid only for media-host→daemon copy
  and allocation pressure. It does not prove WebRTC throughput. RemoteApp media
  is deliberately framed encoded media over RTP/SRTP, not an unstructured raw
  byte pipe; bounded queues and transport-budgeted encoding are part of the
  efficiency contract.
- Focus recovery in the same run committed target focus, but Linux/X11 window
  input remained explicitly `view_only` with
  `target_scoped_keyboard_pointer_dispatch_unsafe`. The Browser runner now
  re-reads post-focus input authority: diagnostic runs report
  `focused_view_only`, while product runs requiring independent host effects
  fail immediately instead of timing out waiting for an impossible applied
  pointer frame.
- The earlier selected-pair-budget implementation and its PERF-08 claim were
  withdrawn after this directionality check. Local RTCP, sender-side TWCC, and
  writer-service pacing now pass focused Rust and mutation gates. A rebuilt
  artifact and sustained real-Browser rerun are still required before PERF-08
  has live evidence. Product completion remains false.

## 2026-08-28 — Shared-slot lifetime root cause and v10 live correction

- A provenance-bound v9 Linux/X11 Browser run reproduced the one-frame stall.
  At 2.115 seconds the media host had captured and encoded 30 frames, while the
  daemon accepted only 2 and dropped 28. The Browser decoded one frame, the
  shared lane entered GOP recovery, and the session failed because no recovery
  IDR reached the daemon within two seconds.
- The mapped payload owner had been passed directly into WebRTC. RTP
  packetization/NACK retention could therefore hold the shared-memory lease
  beyond daemon ingestion, pinning the single negotiated producer slot. This
  was a lifetime-boundary defect, not JSON/base64 overhead and not RTP writer
  service time; the recorded writer p95 was only 0.188 ms.
- The daemon now validates the mapped frame in place, copies it exactly once
  into transport-owned `Bytes`, and immediately releases the shared lease. The
  comparative 128×256 KiB benchmark records 256 allocation calls and
  33,567,744 allocated bytes for shared v2 versus 1,280 calls and 33,612,544
  bytes for payload-pipe v1, with approximately 715.95 versus 714.81 MiB/s in
  that run. The copy is intentional ownership detachment, not a zero-copy
  claim.
- The provenance-bound v10 rerun sustained 1,219 captured, encoded and
  daemon-received frames across 93 seconds with zero daemon/shared-lane drops.
  RTP writer p95 remained approximately 0.61 ms, and neither shared-lane
  recovery nor media failure recurred. This closes the device-side binary-lane
  lifetime defect.
- The Browser product runner still failed independently: its periodic
  `report_client_state` calls arrived after the session lease had expired and
  the UI never exposed visible media-pipeline support. Therefore the run does
  not yet close Browser sustained-decode/readiness acceptance, fresh RTCP
  evidence, or product completion.

## 2026-08-29 — Linux route rerun, platform matrix, and final aggregate

- Direct application passed with 14 rendered frames. TURN passed for window
  and application with relay-only Browser SDP/policy and server-observed
  allocation. EasyNet relay passed for window and exact two-window application
  with Hub-issued ephemeral leases. All accepted leaves used real Browser
  lifecycle evidence and closed with terminal cleanup.
- The EasyNet refresh/resume child passed a real lease rotation, same-session
  daemon restart, replacement connected transport, watch reattachment, nine
  rendered frames, and terminal cleanup.
- Two real STUN binding attempts failed before selected-pair connection. The
  Colima/Docker VM topologies could not provide a viable private return route,
  so the aggregate route matrix still has no STUN srflx proof.
- Linux capture is real for window and process-scoped application, but both
  target kinds correctly remain `view_only`; X11/XTest does not provide the
  target-isolated press-to-release semantics required for Window/Application
  input. macOS is blocked by Screen & System Audio Recording permission for the
  exact signed helper. Windows has no real-host evidence.
- `target/e2e/remoteapp-product-completion/final-20260829/report.json` reports
  `status=failed`, 20 errors, `product_complete_eligible=false`,
  `finalization_state=not_eligible`, and `product_complete_claim=false`. The
  missing authority/report set is explicit; this tree is not eligible for
  merge review under the product-completion rule.

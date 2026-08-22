# Decisions and Deltas — RemoteApp Product Closure

## 2026-08-22 — Session terminal facts must be explicit

Decision:

- RemoteApp session lifecycle needs a deterministic terminal fact for product
  UI and E2E assertions.
- The terminal fact belongs to the RemoteApp plugin session aggregate; it must
  not redefine or replace Axon Invocation receipts.
- Idempotent `end_session` on an already terminal row must return the original
  terminal fact.

Implementation delta:

- `RemoteDesktopSession` now stores one `terminal_receipt` projection.
- Explicit close and lease timeout populate the projection from the stored
  terminal `SESSION_CLOSED` event.
- Public session views expose `terminal_receipt`; active sessions project
  `null`.
- Product closure gates now reject removal of terminal receipt projection and
  idempotent end-session receipt coverage.

Product effect:

- Frontend and E2E code can assert explicit close and timeout outcomes without
  scanning the event log or guessing from `end_reason`.
- This does not implement reconnect/session resume, consent-revoke E2E, or
  crash/restart recovery.

## 2026-08-22 — Application capture must not widen beyond committed window set

Decision:

- An application RemoteApp session captures the committed display-scoped
  `AppWindowSetProof`.
- ScreenCaptureKit application filters are acceptable only if same-app windows
  outside the committed set are explicitly excluded.
- A newly opened same-application window must not silently join an existing
  RemoteApp application session.

Implementation delta:

- The macOS ScreenCaptureKit target resolver now carries native
  `exceptingWindows` with the committed application window-set proof.
- The selector collects same-application, same-display windows that are not in
  the committed proof and passes them to the native filter.
- The target-binding boundary gate and source tests reject empty
  `exceptingWindows`, missing uncommitted-window collection, and missing
  regression coverage.

Product effect:

- This closes a concrete same-app leakage seam for macOS application capture.
- It does not complete Windows/Linux application capture, multi-display
  `MultiAppSurface`, or real app/window churn E2E.

## 2026-08-22 — v8 raw stream must preserve canonical lifecycle metadata

Decision:

- Raw bytes are an ABI v8 transport representation, not a new Invocation or
  stream lifecycle semantic.
- SDKs must reject v8 raw metadata that omits canonical lifecycle, receipt,
  terminal, or error fields.
- ABI v7 remains JSON/base64 compatible.

Implementation delta:

- Python SDK `RawStreamPacket` parsing now requires `sequence`, `kind`,
  `state`, `terminal`, `transport_terminal`, `payload_content_type`,
  `admission_receipt`, `terminal_receipt`, and `error`.
- Rust FFI stream transport-error and receipt-verification-error metadata now
  carry the same v8 metadata fields.
- The v8 ABI gate now checks for SDK strictness and Rust metadata tests.

Product effect:

- EasyRemote/RemoteApp can consume raw media bytes without bypassing Runtime
  Core stream state machines.
- This is data-plane infrastructure only; it does not complete real host
  audio/video capture, codec negotiation, network adaptation, or E2E evidence.

## 2026-08-22 — Host audio must be explicit unsupported product state

Decision:

- RemoteApp video transport readiness must not imply host audio readiness.
- Until the plugin owns real host-audio capture, encode, and WebRTC send
  paths, host audio is a first-class unsupported product state, not an omitted
  field.

Implementation delta:

- Device capability views expose an `audio` object and a `host_audio`
  unsupported capability with the stable reason `host_audio_not_implemented`.
- Session views expose the same `audio` object.
- Production readiness now states `media_scope=video_only`,
  `audio_ready=false`, and `audio_blocked_reason=host_audio_not_implemented`.
- The RemoteApp performance boundary gate pins the projection and mutation
  tests reject false audio readiness.

Product effect:

- Frontend and E2E harnesses can no longer treat video readiness as full
  audio/video readiness.
- This does not implement audio capture, audio codec negotiation, or audio
  E2E; those remain required before product completion.

## 2026-08-22 — User Service projection conflict must not kill Device session

Decision:

- `service/<user>.pages` remains a user-scoped Service owner projection.
- The Hub read model currently selects one live projection row per
  `owner_ura`.
- Equal generation/revision projection conflicts are read-model selection
  outcomes, not authority failures.
- Device-native RemoteApp abilities remain SystemAgent-owned and must not be
  taken offline by a non-selected user Service projection.

Implementation delta:

- `federation.advertise_abilities` responses now carry an optional projection
  upsert `outcome`.
- Strict projection callers still require `ack=true` and exact `count`.
- User-scoped Service owner prelude degrades only when the admitted write is a
  read-model rejection such as `ignored_stale` or `rejected_conflict`.
- Admission, signer delegation, descriptor integrity, transport errors, and
  acknowledged count mismatches still fail closed.

Product effect:

- Cross-device RemoteApp smoke should no longer report the caller Device as
  offline merely because another host already owns the selected Pages Service
  projection.
- This does not claim product completion for real OS capture, input injection,
  audio/video, NAT/relay, or frontend end-to-end RemoteApp lifecycle.

## 2026-08-22 — Cross-device smoke must produce bounded environment evidence

Decision:

- Cross-device RemoteApp evidence must be terminal and inspectable even when
  the local Docker or filesystem environment is not ready.
- A Docker probe hang and insufficient report filesystem space are environment
  failures, not RemoteApp feature failures.

Implementation delta:

- The cross-device smoke now checks report filesystem free space before child
  E2Es.
- The Docker readiness probe uses a bounded `docker info` timeout.
- Each child E2E step runs under a bounded timeout and writes a failed step
  report on timeout/failure.

Product effect:

- Future `--run` attempts will either produce cross-device product evidence or
  a structured failed report explaining why the environment could not execute
  the product path.
- This closes an evidence-chain seam only; it does not complete real capture,
  input, audio/video, relay, or frontend lifecycle coverage.

## 2026-08-22 — Product closure state must be machine-readable

Decision:

- RemoteApp product completion cannot be inferred from Markdown prose, local
  source gates, or synthetic reports.
- The eight product requirements are now represented as an explicit readiness
  matrix with row ids, current evidence, required evidence, and non-claims.
- The matrix is allowed to contain only `partial` or `incomplete` rows until
  real authoritative product evidence exists for every requirement.

Implementation delta:

- Added `docs/design/remoteapp-product-readiness-matrix.json`.
- Extended `check-remoteapp-product-closure-audit.sh` to reject missing matrix
  rows, empty evidence, unsupported statuses, and premature
  `product_complete=true`.
- Extended the audit script tests to mutate the matrix and prove those failure
  modes are caught.

Product effect:

- Future RemoteApp work has a concrete acceptance ledger for real OS capture,
  input, audio/video, multi-window tracking, recovery, network fallback,
  frontend lifecycle, and cross-device E2E.
- This does not implement the missing product capabilities; it prevents
  architectural drift and false product-complete claims while those
  capabilities are built.

## 2026-08-22 — Input readiness must be projected as one product state

Decision:

- A session that was requested as `interactive` but is effectively `view_only`
  must not require callers to infer that state from scattered `mode`,
  `scope_audit`, and `input_policy` fields.
- The RemoteApp plugin should project a single product-level input readiness
  object while keeping actual OS input injection disabled until focus,
  coordinate, permission, and target-epoch proofs exist.

Implementation delta:

- Session views now include top-level `input_readiness`.
- The input plane carries the same readiness object next to its data-channel
  policy.
- The readiness projection reports requested mode, effective mode,
  `interactive_ready`, input scope, pointer/keyboard booleans, and a stable
  blocked reason.
- The lifecycle/input boundary gate and mutation tests now pin this projection.

Product effect:

- Frontend and E2E harnesses can distinguish "user requested interactive but
  product correctly downgraded to view-only" from a truly interactive session.
- This does not complete input injection; it makes the missing interactive
  capability explicit and machine-readable.

## 2026-08-22 — Frontend input sending must consume runtime readiness

Decision:

- The browser must not independently infer interactive input eligibility from
  legacy `input_policy` when the daemon already projects authoritative
  `input_readiness`.
- `input_policy` remains a compatibility projection for sessions that do not
  expose the new readiness object, but new RemoteApp sessions should be gated
  by runtime readiness first.

Implementation delta:

- Frontend session projection now parses daemon `input_readiness`.
- `RemoteDesktopView` carries the parsed input readiness beside the legacy
  input policy.
- `remoteDesktopInputFrameAllowed` fails closed when
  `interactive_ready=false`, and separately gates pointer/wheel and key/keyboard
  frames from daemon `pointer_enabled` and `keyboard_enabled`.
- The frontend boundary checker and its mutation tests now require the parser,
  projection, fail-closed gating, and protocol test coverage.

Product effect:

- A session requested as interactive but downgraded to view-only is now blocked
  consistently at the browser input-sending boundary.
- This closes a frontend/runtime seam only. It does not implement focus-safe
  OS pointer/keyboard injection or provide latency/product E2E evidence.

## 2026-08-22 — Input control consent must be explicit and scope-bound

Decision:

- Remote desktop media/session consent is not sufficient authority for
  pointer, wheel, or keyboard control.
- Display-global input can be enabled only when the consumed local consent
  ticket explicitly carries `input_control=true`.
- Window/application input remains view-only until a target-scoped
  focus/activation validator can prove keyboard and pointer dispatch target the
  selected surface.

Implementation delta:

- `remote_desktop.grant_consent` accepts optional `input_control`.
- The consent registry persists `input_control_granted` in the one-use ticket
  and projects it into the consumed authorization.
- Session consent stores and audits the input-control grant scope.
- `create_session` passes the consumed grant scope into target binding
  resolution.
- Display targets with explicit input-control consent may resolve
  `display_global` input scope; media-only display consent still resolves
  `view_only`.
- Session readiness reports `effective_mode=interactive` only when runtime
  input is actually ready; missing OS accessibility/input permission remains
  `input_injection_unavailable`.
- `check-remoteapp-input-consent-boundary.sh` pins this full source contract.

Product effect:

- The product now has a correct authority gate for display-level interactive
  input instead of treating all interactive requests as permanently view-only.
- This still does not prove product-level input injection. Required remaining
  evidence: macOS Accessibility permission E2E, pointer/wheel/key application
  E2E, latency measurements, target epoch checks on the execution path, and a
  separate safe design for window/application focus-scoped input.

## 2026-08-22 — Frontend must request the input-control consent scope it intends

Decision:

- The frontend's Interactive toggle is a session intent, not proof that input
  is available.
- That intent must still be carried consistently into `grant_consent` and
  `create_session`; otherwise the daemon can support scoped input-control
  consent while browser-created sessions continue to mint media-only tickets.
- Runtime `input_readiness` remains the authority for whether pointer/keyboard
  frames may actually be sent.

Implementation delta:

- The EasyNet frontend now derives a single RemoteApp session input intent and
  uses it for `grant_consent.args.input_control`, `create_session.args.mode`,
  and `create_session.args.input_policy`.
- Frontend store/UI tests assert default interactive sessions request
  `input_control=true`.
- Frontend store tests assert disabled Interactive mode requests
  `input_control=false` and creates a view-only keyboard/pointer policy.
- `check-remoteapp-frontend-invocation-boundary.sh` now gates this contract and
  its mutation self-test rejects drift back to independently-derived grant and
  create parameters.

Product effect:

- Browser-created display RemoteApp sessions can now present the daemon with
  the explicit input-control consent needed to unlock display-global
  interactivity when OS input injection is ready.

## 2026-08-22 — Runtime input readiness must be visible in the session UI

Decision:

- The frontend must distinguish requested interactive intent from daemon
  effective input readiness.
- A session details panel that only says `input interactive` hides the important
  product fact when the daemon downgraded the session to view-only or blocked
  input for OS permission/scope reasons.

Implementation delta:

- Remote Desktop session details now render a daemon-readiness label derived
  from `RemoteDesktopView.inputReadiness`.
- The label shows requested-to-effective mode changes and `blockedReason`, with
  legacy `inputPolicy` used only when `inputReadiness` is absent.
- Frontend component tests and the frontend invocation boundary checker now
  require visible blocked-readiness coverage.

Product effect:

- An interactive RemoteApp request that becomes view-only is now visible to the
  user/operator with the daemon reason.
- This closes the UI observability seam only. Product completion still requires
  real OS input injection, focus/activation safety, latency, and cross-device
  E2E evidence.
- View-only remains safe and explicit.
- This does not claim product-level input injection completion; real OS
  pointer/keyboard E2E and latency evidence remain required.

## 2026-08-22 — Input scope and concrete controls must be visible

Decision:

- Requested/effective input mode and concrete input controls are separate
  product facts.
- A session that only shows `input interactive` or `input interactive->view_only`
  still hides whether pointer and keyboard are enabled and what daemon input
  scope the session is using.

Implementation delta:

- Remote Desktop session details now render `input scope <scope> · <controls>`
  from `RemoteDesktopView.inputReadiness`.
- Component coverage proves both enabled controls
  (`input scope display_global · pointer+keyboard`) and blocked controls
  (`input scope display_global · no controls`).
- The frontend product-flow gate rejects UI or test coverage that drops this
  scope/control visibility.

Product effect:

- Operators can distinguish display-global interactive readiness from blocked
  or non-control sessions without reading raw daemon JSON.
- This is not an OS input injection completion claim; it keeps the input row
  incomplete until successful focus-safe injection and latency evidence exists.

## 2026-08-22 — Permission recovery must include input injection

Decision:

- `remote_desktop.request_permission` is a host-local permission recovery
  ability for RemoteApp, not a target-resource action.
- Its daemon result includes both Screen Recording and
  Accessibility/input-injection permission state, so the public descriptor and
  frontend status must not present it as Screen Recording-only.

Implementation delta:

- Updated the request-permission descriptor/schema description to include
  Accessibility for pointer/keyboard input injection.
- Frontend `rdRequestPermission` now parses daemon `input_permission` and
  includes the input permission outcome in visible status/error text.
- The RemoteApp action row now offers `Request permission` when session
  `inputReadiness.blockedReason` reports `input_injection_unavailable`.
- Frontend invocation and product-flow gates reject dropping the structured
  input permission handling or the executable recovery CTA.

Product effect:

- Users can recover from input-permission blockers from the session UI instead
  of seeing only a passive `input_injection_unavailable` badge.
- This remains permission-recovery evidence only; focus-safe OS injection,
  coordinate mapping, and latency E2E remain required.

## 2026-08-22 — Share picker needs host-local permission preflight

Decision:

- Authorization is a product step before session creation, not only an error
  handler after capture or create-session fails.
- `remote_desktop.permission_status` is the correct host-local, non-prompting
  preflight ability; it must not be scoped to the selected target resource.

Implementation delta:

- Added frontend `rdCheckPermission` that invokes
  `remote_desktop.permission_status` with `args: {}` and no target subject.
- The share picker exposes `Check permissions` and renders the structured
  Screen Recording plus Accessibility/input permission status without leaving
  the picker.
- Frontend invocation/product-flow gates and tests reject target-scoped
  permission_status calls or missing preflight UI coverage.

Product effect:

- Users can verify host permission readiness before starting a RemoteApp
  session, keeping the picker → permission → consent → create flow explicit.
- This remains preflight evidence only; successful OS input injection and real
  Browser/Tauri E2E are still required for product completion.

## 2026-08-22 — Denied permission preflight must stay in picker

Decision:

- `permission_status` is an authorization preflight, not a session creation
  failure.
- A denied preflight must not set the global RemoteApp entry error because that
  exits the picker and breaks the user path to `request_permission`.

Implementation delta:

- `rdCheckPermission` now writes visible status but leaves `entry.error`
  undefined.
- The share picker displays denied preflight status and an inline
  `Request permission` action.
- Component/store tests and product-flow gates prove denied preflight keeps the
  picker open.

Product effect:

- Users stay in the same picker context after a failed permission preflight and
  can immediately request the missing host permission.
- This improves authorization flow correctness only; product completion still
  requires real OS and cross-device E2E evidence.

## 2026-08-22 — Frontend must preserve RemoteApp terminal receipts

Decision:

- A RemoteApp session terminal fact is product lifecycle state, not an Axon
  Invocation receipt.
- The frontend must not erase that fact by replacing a successful
  `end_session` response with `session=null`.

Implementation delta:

- `RemoteDesktopView` now carries the parsed daemon `terminal_receipt`
  projection.
- `rdEnd` retains a closed terminal view, clears `sessionToken`, and marks the
  local attachment false.
- Session details render the terminal reason and event sequence.
- The frontend boundary checker and mutation self-test reject missing terminal
  receipt projection, missing UI coverage, and regressions back to
  `session=null`.

Product effect:

- Users and E2E harnesses can distinguish a known terminal session from a
  missing session after local transport teardown.
- This closes a frontend lifecycle observability seam only. Session resume,
  reconnect, consent-revoke termination E2E, and crash/restart recovery remain
  required before product completion.

## 2026-08-22 — Permission revocation is a terminal RemoteApp outcome

Decision:

- A host permission revocation invalidates the consent grant for the current
  RemoteApp session. The existing session must not remain suspended and
  lease-refreshable under the old grant.
- The consent state remains `revoked`, not `expired`, so audit can distinguish
  caller close / lease expiry from user or platform permission revocation.

Implementation delta:

- Added stable terminal reason `target_permission_revoked`.
- `TargetObservation::PermissionRevoked` now revokes consent, emits
  `TARGET_PERMISSION_REVOKED`, emits `MEDIA_SOURCE_LOST`, and closes the
  session with `SESSION_CLOSED` plus a RemoteApp `terminal_receipt`.
- The frontend marks permission-revoked recovery as terminal-sync-required,
  closes local WebRTC/input transport, invokes `remote_desktop.show_session`,
  retains the daemon terminal receipt, and clears the session token.
- Lifecycle and frontend boundary gates now reject regressions back to
  suspended-only revoke handling.

Product effect:

- Permission revoke no longer leaves a zombie RemoteApp session occupying lease
  lifecycle while the UI tells the user to create a new session.
- This still does not prove real OS permission-revoke E2E, reconnect/resume, or
  crash/restart recovery.

## 2026-08-22 — Target tracker input loss must block session input readiness

Decision:

- Runtime `input_readiness` must reflect the same target-state predicate used
  by input execution.
- A session whose latest target snapshot has `input_enabled=false` must not
  report `interactive_ready=true`, even if the requested mode, consent, and OS
  input-injection permission would otherwise allow input.

Implementation delta:

- `input_readiness_view` now returns
  `blocked_reason=target_input_not_ready` before OS input-permission checks
  when `session.target_snapshot().input_enabled()` is false.
- Added a daemon session-view regression for display interactive input-control
  consent followed by target loss.
- Extended the input-consent boundary gate and mutation self-test so this
  target-state readiness dependency cannot be dropped.

Product effect:

- The public session view, frontend UI, and input data-channel execution path
  now agree when target tracking disables input.
- This is still a safety/projection closure only; successful low-latency
  pointer/keyboard injection remains unproven.

## 2026-08-22 — Pointer input must reject stale target geometry

Decision:

- Target-local pointer input is unsafe if the browser sends coordinates derived
  from an older target geometry than the daemon's committed target tracker
  snapshot.
- When an effective input policy carries a pointer target
  `target_geometry_revision`, pointer frames must echo that revision and the
  daemon must reject mismatches before OS injection.
- Display-global input remains allowed to omit the field because no target-local
  geometry transform is used.

Implementation delta:

- `PointerInputFrame` now accepts optional `target_geometry_revision`.
- `apply_input_frame_with_effective_policy` rejects stale/missing target-local
  pointer revisions with `stale_pointer_target_geometry` before platform
  dispatch.
- The frontend pointer frame builder includes
  `entry.session.targetTracking.targetGeometryRevision` when present.
- Lifecycle/input and frontend boundary gates now pin the source and tests.

Product effect:

- This closes a real execution-path safety seam for future target-local pointer
  dispatch and prevents stale client coordinates from reaching CGEvent/X11/etc.
- The input-injection product row remains incomplete until real OS E2E and
  latency evidence exist.

## 2026-08-22 — v8 raw-stream ABI must ship in release shape

Decision:

- `runtime_invocation_stream_open_v8` is an additive raw-payload transport
  representation for high-frequency media streams, not a new Invocation
  semantic.
- If the header and feature discovery expose v8, release packages must ship the
  v8 export allowlist beside the base v7 allowlist. Otherwise downstream SDKs
  can compile against a source-tree contract that is not auditable after
  installation.
- `runtime_abi_version()` remains `7`; bindings must feature-detect the v8
  symbol before using it.

Implementation delta:

- Release tarball staging now includes `include/easynet_cli.exports.v8`.
- Unix installer, sandbox release-install E2E, and Windows staging now install
  or stage the v8 allowlist.
- ABI, release-package, SDK scaffold, and project-structure gates now require
  the v8 allowlist where the release contract is asserted.

Product effect:

- RemoteApp/EasyRemote raw media stream consumers can verify the installed ABI
  extension contract instead of relying on source-tree-only evidence.
- This closes a distribution seam only; it does not prove codec negotiation,
  host audio, relay behavior, or cross-device RemoteApp media readiness.

## 2026-08-22 — Device presence loss must not terminate RemoteApp sessions

Decision:

- A device online/offline presence change is a transport availability signal,
  not a user/session terminal action.
- The only product-level terminal actions for a RemoteApp session remain
  explicit `end_session`, lease/session timeout, daemon terminal recovery, or
  permission-revoked terminalization with a `terminal_receipt`.
- Therefore the frontend must not call `rdEnd` or clear a non-terminal
  RemoteApp session merely because the selected device becomes temporarily
  offline.

Implementation delta:

- Frontend offline suspend preserves non-terminal RemoteApp session state and
  closes only local browser transport.
- Frontend online resume validates the preserved session with
  `remote_desktop.show_session`, rebinds WebRTC, restarts `watch_events`, and
  refreshes the lease.
- Resume-time WebRTC transport failure preserves the daemon session instead of
  invoking end-session cleanup.
- The CLI frontend boundary gate now rejects session clearing in
  `suspendEntryForOffline`, missing `show_session` validation during resume,
  resume rebinds that end the daemon session on transport failure, and
  `DeviceMediaAccess` offline effects that call `rdEnd`.

Product effect:

- A short UI/device presence drop no longer destroys a valid daemon RemoteApp
  session, so the user can reconnect without losing session lifecycle state.
- This closes the frontend offline/resume seam only; long outages, NAT/relay
  handoff, process crash/restart, and real cross-device recovery still require
  E2E evidence before RemoteApp can be called product-complete.

## 2026-08-22 — Input rejection must be visible at the product surface

Decision:

- A browser data-channel send returning `true` does not prove OS input
  injection. The daemon may still reject the frame because input permission,
  policy, target tracking, target geometry revision, or platform support is not
  ready.
- The daemon already owns this execution decision and records it as RemoteApp
  session events. The frontend must consume those events instead of inventing a
  second input-result protocol.
- Ordinary input rejection is not a media transport failure and must not close
  WebRTC by default.

Implementation delta:

- Frontend `watch_events` recovery now maps `INPUT_CHANNEL_OPENED` with
  blocked activation into visible input-blocked status.
- Frontend `watch_events` recovery now maps `INPUT_FRAME_REJECTED` into visible
  input-rejected status including daemon reason and frame kind/action when
  present.
- The frontend boundary gate rejects missing input rejection/activation event
  handling and rejects treating input rejection as a default media transport
  close.

Product effect:

- Users and E2E harnesses can observe why input did not take effect, including
  reasons such as `input_injection_unavailable` and
  `stale_pointer_target_geometry`.
- This closes a silent-failure seam only; successful low-latency pointer and
  keyboard injection still needs real OS E2E evidence across supported
  platforms.

## 2026-08-22 — Frontend input timestamps are part of the RemoteApp data-channel schema

Decision:

- The browser currently sends `sent_at_ms` on RemoteApp pointer/key frames.
  Because the daemon parser uses `deny_unknown_fields`, that field must be an
  explicit part of the RemoteApp input frame schema or real frontend input will
  be rejected before policy and OS injection.
- The field is input-plane observability metadata, not a new Axon Invocation
  tuple field and not an authority decision.
- The daemon must preserve strict unknown-field rejection for all other schema
  drift.

Implementation delta:

- `PointerInputFrame` and `KeyInputFrame` accept optional `sent_at_ms`.
- The daemon bounds the value to JavaScript-safe integer range.
- Input applied/rejected session events preserve the value as
  `client_sent_at_ms` when present.
- Lifecycle/input and frontend boundary gates now pin the cross-repo schema
  contract: frontend attaches `sent_at_ms`, daemon accepts it, and events
  retain it for observability.

Product effect:

- Real frontend pointer/key input can now reach daemon policy and OS-injection
  decisions instead of being rejected as an invalid frame solely because of
  frontend metadata.
- This is a required precondition for trustworthy input latency E2E, but it
  does not itself prove successful low-latency OS injection.

## 2026-08-23 — RemoteApp input send must be bounded and sequenced

Decision:

- High-rate pointer input is product data-plane traffic. The browser must not
  keep enqueueing stale frames into a backed-up RTC data channel because that
  converts remote control into delayed control.
- The sequence is frontend telemetry only. It does not replace daemon session
  authority, transport epoch checks, target geometry revision checks, or Axon
  receipts.
- The daemon plugin must parse and project the sequence because host-side
  applied/rejected events are the authoritative place to diagnose whether a
  browser-sent frame reached input policy and OS injection.

Implementation delta:

- Frontend `rdSendInput` now refuses sends when `RTCDataChannel.bufferedAmount`
  exceeds the explicit RemoteApp input bound.
- Accepted frontend input frames carry monotonic `client_sequence` and
  `sent_at_ms`.
- The remote-desktop plugin accepts optional `client_sequence` on pointer/key
  frames, validates it as a non-zero JavaScript-safe integer, and includes it
  in applied/rejected input events.
- Frontend and lifecycle boundary gates now pin the backpressure and sequence
  contract.

Product effect:

- A congested RemoteApp input channel now fails closed with visible
  backpressure state instead of silently accumulating stale mouse/keyboard
  input.
- Host-side event logs can correlate browser send order and timestamp with
  daemon applied/rejected decisions. This improves latency/loss diagnosis, but
  still does not prove successful cross-platform OS input injection.

## 2026-08-23 — Diagnostic bidi input must preserve client telemetry

Decision:

- Diagnostic InvokeBidi input exists to probe RemoteApp session/input behavior
  without defining a separate product input API. It must therefore use the same
  parser and effective-policy object as the production WebRTC data channel.
- Probe correlation still needs the browser's `client_sequence` and
  `sent_at_ms` metadata. Dropping those fields from diagnostic applied/warn
  responses makes it impossible to correlate a probe frame with daemon policy
  rejection or target readiness.
- The telemetry is observational only; session authority, target binding,
  transport epoch, and input readiness remain daemon-owned.

Implementation delta:

- Diagnostic bidi input responses now project `client_sent_at_ms` and
  `client_sequence` when present.
- `target_input_not_ready` diagnostic responses also preserve telemetry, so
  window/application target loss can be correlated with the exact client input
  frame.
- Lifecycle/input boundary gates now require diagnostic bidi telemetry
  projection alongside production data-channel telemetry.

Product effect:

- Host probes can correlate diagnostic input requests with daemon policy and
  target-readiness decisions instead of observing generic warnings detached
  from the client frame.
- This improves executable evidence quality for input safety and recovery, but
  still does not prove successful real OS pointer/keyboard injection.

## 2026-08-23 — Host view-only input E2E must exercise public Bidi input

Decision:

- A session view that says `input_scope=view_only` is not enough product
  evidence. The public input transport must also reject pointer/key frames
  under the same policy.
- The correct boundary for this host evidence is
  `easynet ability bidi remote_desktop.attach` with explicit `subject`,
  nonce, and causal root, not a private Rust helper.
- Pointer/key probe frames must carry frontend-shaped `sent_at_ms` and
  `client_sequence` so host E2E evidence can correlate browser input with
  daemon rejection.

Implementation delta:

- `host-remoteapp-view-only-input-safety-e2e.sh` now opens the public
  `remote_desktop.attach` InvokeBidi path after session creation and sends
  pointer/key/close frames.
- The harness validates that view-only app/window sessions do not emit
  `input_applied` and that pointer/key warnings are
  `input_scope_unsupported` with preserved `client_sent_at_ms` and
  `client_sequence`.
- The E2E acceptance checker now rejects host view-only input harnesses that
  omit the public Bidi probe or the client telemetry checks.

Product effect:

- RemoteApp now has stronger executable evidence that app/window sessions fail
  closed on the real diagnostic input transport while preserving correlation
  metadata.
- This still does not prove product-complete native input injection; the
  remaining requirement is successful focus-safe OS pointer/keyboard injection
  E2E with latency and permission evidence.

## 2026-08-23 — Session timeout needs host-level terminal evidence

Decision:

- Lease timeout is part of RemoteApp product lifecycle, not only an internal
  timer. Product evidence must observe timeout through public CLI/daemon
  session views.
- A timeout terminal receipt must be stable. Calling `end_session` after
  timeout should be idempotent and preserve the original `session_expired`
  receipt instead of creating a second close fact.

Implementation delta:

- Added `host-remoteapp-session-timeout-e2e.sh`.
- The harness selects a live Resource URA, creates a short-lived
  `remote_desktop.create_session`, waits past the lease, observes the closed
  `session_expired` state through `remote_desktop.show_session`, then invokes
  `remote_desktop.end_session` and verifies `already_ended=true`.
- The E2E acceptance gate now requires the timeout harness, short-lease
  creation, public show-session observation, `terminal_receipt.reason_code`,
  and idempotent receipt preservation.

Product effect:

- Timeout lifecycle now has host-level executable evidence instead of only
  unit/handler evidence.
- This still does not prove long-outage reconnect, crash/restart recovery,
  consent revoke E2E, or cross-device timeout receipt chains.

## 2026-08-23 — Session cancel needs host-level terminal evidence

Decision:

- RemoteApp product cancel is the user/session close path exposed by
  `remote_desktop.end_session`; it is not the Axon transport-level
  `invocation.cancel` lifecycle primitive.
- A user cancel terminal receipt must be stable. Calling `end_session` again
  after cancel should be idempotent and preserve the original `user_cancelled`
  receipt instead of creating a second close fact.

Implementation delta:

- Added `host-remoteapp-session-cancel-e2e.sh`.
- The harness selects a live Resource URA, creates a
  `remote_desktop.create_session`, invokes public `remote_desktop.end_session`
  with `user_cancelled`, observes the closed session through
  `remote_desktop.show_session`, then invokes `remote_desktop.end_session`
  again and verifies `already_ended=true`.
- The E2E acceptance and product-closure gates now require the cancel harness,
  public end-session invocation, public show-session observation,
  `terminal_receipt.reason_code`, and idempotent receipt preservation.

Product effect:

- User-initiated close/cancel lifecycle now has host-level executable evidence
  instead of only handler/unit evidence.
- This still does not prove transport-level `invocation.cancel`,
  long-outage reconnect, crash/restart recovery, consent revoke E2E, or
  cross-device cancel receipt chains.

## 2026-08-23 — Permission revoke needs a live host evidence harness

Decision:

- RemoteApp permission revoke must be proven as a product session terminal
  outcome observed through public `remote_desktop.show_session`.
- The E2E must not add or rely on a debug revoke ability. Live product evidence
  requires a real platform/operator permission revoke; self-test can validate
  only the harness evidence contract.

Implementation delta:

- Added `host-remoteapp-permission-revoke-e2e.sh`.
- The harness creates a live-target `remote_desktop.create_session`, waits for
  real platform permission revocation, and accepts only a public
  `remote_desktop.show_session` projection with `target_permission_revoked`,
  revoked consent, ordered `TARGET_PERMISSION_REVOKED`,
  `MEDIA_SOURCE_LOST`, `SESSION_CLOSED` events, and a terminal receipt bound
  to the created session id.
- The E2E acceptance and product-closure gates now require the harness, real
  platform proof mode, operator/platform revoke requirement, public
  show-session observation, event evidence, and
  `terminal_receipt.reason_code`.

Product effect:

- Permission-revoke closure now has an executable host harness that can collect
  real OS evidence without weakening plugin/runtime boundaries.
- Product completion still requires an actual live pass report from a real host
  permission revoke, plus reconnect/resume, crash/restart, and cross-device
  revoke evidence.

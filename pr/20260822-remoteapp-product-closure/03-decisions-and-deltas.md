# Decisions and Deltas — RemoteApp Product Closure

## 2026-08-25 — Input frames and pressed state require one lifecycle contract

Decision:

- The browser may send only fields accepted by the plugin's strict input-frame
  parser. DOM-only facts such as `buttons` and `pointer_type` do not belong on
  the RemoteApp wire when no host behavior consumes them.
- A successfully injected key/button down becomes transport-owned pressed
  state. Its matching release is a reducing operation and must not be blocked
  by a later focus/geometry change.
- Browser blur/pointer cancellation releases known local presses promptly;
  device-side channel termination remains the authoritative cleanup for
  disconnects where the browser cannot deliver a final frame.

Implemented product effect:

- Real pointer frames reach policy, coordinate mapping, and OS injection
  instead of failing JSON deserialization on unknown frontend fields.
- Focus loss, transport retry, and abrupt disconnect cannot intentionally leave
  a tracked modifier or mouse button held on the host.

Verification completed on 2026-08-25:

- EasyNet frontend contract tests prove exact pointer serialization plus
  blur/pointer-cancel release behavior; the full frontend suite passed with
  81 files and 689 tests.
- Frontend typecheck, focused lint, and production build passed.
- Plugin input state-machine tests passed with 37 focused tests, covering
  bounded tracking, reducing releases, and terminal cleanup.
- RemoteApp platform-input, frontend-invocation, lifecycle-input, and product
  closure static gates passed.
- The main-crate implementation gate passes from an isolated worktree at this
  commit. The original shared checkout remains temporarily non-compilable due
  to concurrent, unrelated bidi/file-transfer changes (`BidiInputFrame`
  call-site drift and a missing `PTY_CONTROL_STREAM_ID`), which were excluded
  from this delta.
- A Windows GNU cross-build reached the OpenH264/Nokhwa native archive stage
  without a Rust type error, then stopped because the host volume ran out of
  space. Windows/Linux cross-builds therefore remain unverified rather than
  being counted as passed.
- Real cross-device macOS/Windows/Linux OS-input E2E remains required before
  the RemoteApp product can be called complete.

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
- The Hub read model must not collapse all Service host placements into one
  owner-only row.
- Equal generation/revision projection conflicts are read-model selection
  outcomes for single-owner rows, not authority failures.
- Device-native RemoteApp abilities remain SystemAgent-owned and must not be
  taken offline by a non-selected user Service projection.

Implementation delta:

- `federation.advertise_abilities` responses now carry an optional projection
  upsert `outcome`.
- Strict projection callers still require `ack=true` and exact `count`.
- Service owner projections are now fenced per `(owner_ura, host_device_ura)`;
  user-scoped Service abilities can be published by multiple paired host
  Devices without false same-owner conflicts.
- Service route resolution selects a live host Device row while keeping the
  Service as callee/owner and keeping Service rows out of Agent/SystemAgent
  directory listing.
- Admission, signer delegation, descriptor integrity, transport errors, and
  acknowledged count mismatches still fail closed.

Product effect:

- Cross-device RemoteApp smoke should no longer fail at the historical
  `accepted_count=0, expected_count=5` Pages Service projection conflict.
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

## 2026-08-23 — RemoteApp implementation tests must run through the main crate

Decision:

- The standalone `easynet-plugin-remote-desktop` crate is a provider/export
  shim. A green standalone test run with zero tests is not RemoteApp
  implementation evidence.
- Product closure gates must execute implementation tests through the main
  EasyNet crate where the daemon embeds the plugin implementation.
- The gate must fail if a selected test filter matches zero tests.

Implementation delta:

- Added `tools/scripts/check-remoteapp-main-crate-implementation-tests.sh`.
- The script verifies the provider-shim boundary and runs main-crate tests for
  app/window target observation, explicit rebind policy, non-macOS app/window
  fail-closed behavior, WebRTC app/window display-fallback rejection, native
  plugin platform catalogue state, production-vs-diagnostic target-subject
  projection, and current-session input policy.
- The product-closure audit now requires this gate and records it in the plan
  pack.

Product effect:

- This prevents a concrete false-readiness seam: using a zero-test standalone
  package result to claim RemoteApp app/window/input/media implementation
  coverage.
- It does not prove live macOS/Windows/Linux capture, successful OS input
  injection, audio/video adaptation, network fallback, frontend lifecycle, or
  cross-device product completion.

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

## 2026-08-23 — Session resume needs daemon lease-refresh evidence

Decision:

- Short disconnect/resume must preserve the same daemon RemoteApp session; a
  replacement `create_session` is not resume evidence.
- The daemon/session layer can be proven independently from browser WebRTC
  rebind by refreshing the lease, waiting past the original lease, and
  validating the same non-terminal session through public `show_session`.

Implementation delta:

- Added `host-remoteapp-session-resume-e2e.sh`.
- The harness creates a short-lease `remote_desktop.create_session`, invokes
  public `remote_desktop.refresh_lease`, waits past the original lease,
  validates the same non-terminal session and refreshed
  `lease_expires_at_ms` through `remote_desktop.show_session`, and closes the
  session through public `remote_desktop.end_session` with
  `resume_e2e_cleanup`.
- The E2E acceptance and product-closure gates now require the resume harness,
  public refresh/show/end abilities, `lease_refresh_resume` proof mode,
  original-lease survival evidence, explicit lease extension, and cleanup
  terminal reason.

Product effect:

- Daemon/session lease-refresh resume now has host-level executable evidence.
- Product completion still requires browser/WebRTC rebind after resume,
  long-outage reconnect, crash/restart recovery, and cross-device resume
  evidence.

## 2026-08-23 — Browser/Tauri frontend lifecycle needs an artifact verifier

Decision:

- Component tests, store tests, and host-only CLI probes are not full
  Browser/Tauri product evidence.
- The product gate needs a runner-agnostic artifact contract so Playwright,
  Tauri driver, or another real UI runner can prove the same lifecycle without
  adding a frontend package dependency here.

Implementation delta:

- Added `tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh`.
- The verifier accepts either `--evidence-json` or `--runner-cmd` in explicit
  `--run` mode and emits bounded JSON/Markdown reports.
- Evidence must state `proof_mode=real_browser_tauri_lifecycle`,
  `component_mock=false`, `real_backend_runtime=true`, and
  `product_complete_claim=false`.
- Evidence must include ordered app/auth/picker/permission/consent/create/
  attach/watch/media/input/end/terminal-receipt steps, public RemoteApp ability
  names, host-local `remote_desktop.permission_status`, selected Resource URA
  binding for session abilities, and a visible terminal receipt.
- Frontend product-flow and product-closure gates now reject missing verifier
  coverage, component-mock proof mode, or hidden terminal receipt evidence.

Product effect:

- The repository now has a precise contract for the missing frontend product
  lifecycle artifact.
- This still does not prove product completion; a live Browser/Tauri artifact
  against a real backend/runtime remains required.

## 2026-08-23 — Network fallback needs live route artifact verification

Decision:

- Typed route models, ICE server projection, and frontend route labels are
  necessary plumbing, but they do not prove that direct, STUN srflx, TURN relay,
  or EasyNet relay paths are reachable.
- Product evidence must come from a real two-device, network-namespace, or
  deployment runner and must include the selected WebRTC candidate pair,
  rendered media, redacted credentials, public RemoteApp session abilities, and
  terminal receipts for every route scenario.

Implementation delta:

- Added `tools/scripts/remoteapp-network-fallback-e2e.sh`.
- The verifier accepts `--evidence-json` or `--runner-cmd` only in explicit
  `--run` mode and emits bounded JSON/Markdown reports.
- Evidence must state `proof_mode=real_network_fallback_matrix`,
  `component_mock=false`, `real_backend_runtime=true`, and
  `product_complete_claim=false`.
- Evidence must include passing `direct`, `stun_srflx`, `turn_relay`, and
  `easynet_relay` scenarios with selected Resource URA subject binding,
  connected/completed ICE state, route-specific candidate evidence, rendered
  media, and visible terminal receipts.
- The verifier rejects raw credential/secret fields and requires redaction
  markers on all scenarios.
- Product-closure gates now require this verifier and reject missing route
  coverage or missing terminal/media proof.

Product effect:

- The repository now has a precise contract for the missing network fallback
  artifact.
- This still does not prove product completion; live direct/STUN/TURN/EasyNet
  relay artifacts against real backend/runtime infrastructure remain required.

## 2026-08-23 — Cross-platform capture needs live host artifact verification

Decision:

- macOS ScreenCaptureKit code and non-macOS fail-closed behavior are not enough
  to claim product-level application/window capture across platforms.
- The product gate needs a live capture artifact that separates real capture,
  explicit product unsupported state, and invalid display fallback.

Implementation delta:

- Added `tools/scripts/remoteapp-cross-platform-capture-e2e.sh`.
- The verifier accepts `--evidence-json` or `--runner-cmd` only in explicit
  `--run` mode and emits bounded JSON/Markdown reports.
- Evidence must state `proof_mode=real_cross_platform_capture_matrix`,
  `component_mock=false`, `real_backend_runtime=true`, and
  `product_complete_claim=false`.
- macOS must pass display/window/application capture with exact target binding,
  rendered frames, public RemoteApp session abilities, selected Resource URA
  subject binding, and visible terminal receipts.
- Windows/Linux must either pass those scenarios or report
  `explicit_product_unsupported` with `show_unsupported=true`, no capture
  session, no rendered frames, and no first-display fallback.
- Product-closure gates now require this verifier and reject missing platform
  coverage, unsupported macOS capture, or display fallback for window/app
  capture.

Product effect:

- The repository now has a precise contract for the missing cross-platform
  capture artifact.
- This still does not prove product completion; live macOS/Windows/Linux host
  artifacts remain required.

## 2026-08-23 — Input injection needs live host effect verification

Decision:

- Input readiness, input-control consent, stale-geometry rejection, and
  telemetry are necessary, but they do not prove that pointer/keyboard input is
  actually applied by the host OS.
- Product evidence must prove permission correctness, focus safety, coordinate
  mapping, target geometry revision binding, bounded latency, observed OS input
  effect, and deterministic session terminal state.

Implementation delta:

- Added `tools/scripts/remoteapp-input-injection-e2e.sh`.
- The verifier accepts `--evidence-json` or `--runner-cmd` only in explicit
  `--run` mode and emits bounded JSON/Markdown reports.
- Evidence must state `proof_mode=real_input_injection_matrix`,
  `component_mock=false`, `real_backend_runtime=true`, and
  `product_complete_claim=false`.
- macOS must pass pointer and keyboard input injection with granted OS input
  permission, `input_control` consent, `display_global` input scope, focus
  validation, coordinate mapping validation, positive target geometry revision,
  public RemoteApp session abilities, selected Resource URA subject binding,
  `INPUT_FRAME_APPLIED` events, bounded latency, observed OS effects, and
  visible terminal receipt.
- Windows/Linux must either pass or report `explicit_product_unsupported` with
  `show_unsupported=true`.
- Product-closure gates now require this verifier and reject missing keyboard
  or pointer evidence, missing permission, high latency, and product-complete
  claims.

Product effect:

- The repository now has a precise contract for the missing real input
  injection artifact.
- This still does not prove product completion; live host input artifacts remain
  required.

## 2026-08-23 — Media adaptation needs live audio/video data-plane evidence

Decision:

- Existing H.264/WebRTC implementation, frontend media stats, ABI v8 raw stream
  packaging, and synthetic media carrier tests are necessary but insufficient
  for RemoteApp product readiness.
- Product evidence must prove the observed audio/video data plane: negotiated
  video codec, payload content type, transport, FPS, bitrate, host audio,
  bounded queue/backpressure behavior, stale-frame drop policy, adaptation
  under degraded network, rendered media after adaptation, and deterministic
  terminal state.

Implementation delta:

- Added `tools/scripts/remoteapp-media-adaptation-e2e.sh`.
- The verifier accepts `--evidence-json` or `--runner-cmd` only in explicit
  `--run` mode and emits bounded JSON/Markdown reports.
- Evidence must state `proof_mode=real_media_adaptation_matrix`,
  `component_mock=false`, `real_backend_runtime=true`, and
  `product_complete_claim=false`.
- Required scenarios are `baseline`, `degraded_network`, and `backpressure`.
- Each scenario must bind public `remote_desktop.create_session`,
  `remote_desktop.attach`, `remote_desktop.watch_events`, and
  `remote_desktop.end_session` to the selected Resource URA and session id.
- Video evidence must include negotiated codec, payload content type,
  transport, encoded/rendered frame counts, duration, requested/effective/
  measured FPS, target/observed bitrate, keyframe interval, and bounded frame
  latency.
- Audio evidence must pass with negotiated codec, sample rate, channels, and
  rendered packets or samples; `host_audio_not_implemented` is explicitly not
  accepted as product media evidence.
- Degraded-network evidence must include bitrate downshift plus FPS downshift
  or frame drop and rendered frames after adaptation.
- Backpressure evidence must include backpressure detection and positive frame
  drops while queue depth remains bounded.

Product effect:

- The repository now has a precise contract for the missing real audio/video
  media artifact.
- This still does not prove product completion; live media artifacts remain
  required.

## 2026-08-23 — Multi-window tracking needs live stream-isolation evidence

Decision:

- Target tracker state machines, same-display application window-set rebind,
  and ScreenCaptureKit `exceptingWindows` are necessary but insufficient for
  product readiness.
- Product evidence must prove the execution effect: independent live streams
  stay bound to their selected Resource URAs while windows move, resize,
  disappear, rebind, or remain explicitly unsupported.

Implementation delta:

- Added `tools/scripts/remoteapp-multi-window-tracking-e2e.sh`.
- The verifier accepts `--evidence-json` or `--runner-cmd` only in explicit
  `--run` mode and emits bounded JSON/Markdown reports.
- Evidence must state `proof_mode=real_multi_window_tracking_matrix`,
  `component_mock=false`, `real_backend_runtime=true`, and
  `product_complete_claim=false`.
- Required scenarios are `independent_window_streams`, `geometry_churn`,
  `application_window_set_churn`, `target_loss_rebind`, and
  `multi_display_application`.
- Independent stream evidence must show at least two concurrent windows with
  distinct Resource URAs, session ids, stream ids, media source epochs, frame
  source ids, rendered frames, exact target binding, and `frames_interleaved=false`.
- Geometry churn must include ordered `TARGET_MOVED` and `TARGET_RESIZED`
  events with increasing `target_geometry_revision`.
- Application churn must include same-display window-set expansion or
  contraction, `PENDING_MEDIA_REBIND`, `TARGET_REBOUND`, increased binding
  epoch, rendered frames after rebind, and no first-display/display fallback.
- Target loss evidence must include `TARGET_LOST` and either
  `TARGET_REBIND_FAILED` with actionable recovery or `TARGET_REBOUND` with
  rendered frames after rebind.
- Multi-display application evidence must either pass with `MultiAppSurface`
  or report `explicit_product_unsupported` without starting a capture session.

Product effect:

- The repository now has a precise contract for the missing real app/window
  churn and stream-isolation artifact.
- This still does not prove product completion; live tracking artifacts remain
  required.

## 2026-08-23 — Crash/restart recovery needs live runtime recovery evidence

Decision:

- Timeout, cancel, permission revoke, and lease refresh harnesses do not prove
  daemon/plugin crash recovery.
- Product evidence must prove that restart recovery is deterministic through
  public RemoteApp abilities: the same session or the same terminal receipt is
  visible after restart, replay/idempotency/lock guards are restored, and stale
  sockets do not require manual cleanup.

Implementation delta:

- Added `tools/scripts/remoteapp-crash-restart-recovery-e2e.sh`.
- The verifier accepts `--evidence-json` or `--runner-cmd` only in explicit
  `--run` mode and emits bounded JSON/Markdown reports.
- Evidence must state `proof_mode=real_crash_restart_recovery_matrix`,
  `component_mock=false`, `real_backend_runtime=true`, and
  `product_complete_claim=false`.
- Required scenarios are `daemon_restart_active_session`,
  `plugin_worker_restart`, `terminal_receipt_replay_after_crash`, and
  `stale_socket_restart_cleanup`.
- Daemon restart evidence must prove unclean process stop, restart,
  `SESSION_REHYDRATED`, stable session id, selected Resource URA, descriptor
  version, target binding epoch, transport epoch, public `show_session`,
  `watch_events` reattach, media reattach, post-restart rendered frames, and a
  cleanup terminal receipt.
- Recovery guards must prove WAL replay, idempotency state recovery, replay
  guard recovery, lock-owner recovery, no duplicate invocation replay, and
  increasing restart epoch.
- Plugin worker restart evidence must prove worker/target-monitor restart,
  same public session, media source epoch advancement, post-restart frames, and
  no new consent minting.
- Terminal-receipt crash evidence must prove original terminal receipt replay,
  closed `show_session` state, and idempotent repeated `end_session`.
- Stale socket evidence must prove explicit stale control/invocation socket
  detection, endpoint readiness after restart, and no manual cleanup.

Product effect:

- The repository now has a precise contract for the missing live runtime
  crash/restart recovery artifact.
- This still does not prove product completion; live recovery artifacts remain
  required.

## 2026-08-24 — Permission readiness must describe the executable host backend

Decision:

- A permission probe is product state, not a platform-neutral success flag.
- The daemon must not call Windows/Linux input permission “Accessibility”, must
  not report Wayland capture ready before a portal backend exists, and must not
  claim that it requested an OS prompt when the platform has no request API.

Implementation delta:

- Added a cohesive input permission probe that binds permission identity,
  backend, availability, requestability, and typed unavailable reason.
- Linux screen capture now fails closed for Wayland and a missing X11
  `DISPLAY`; unknown platforms also fail closed instead of inheriting the
  Windows success baseline.
- Frontend permission labels now render the platform-specific daemon contract.

Product effect:

- Users receive an actionable explanation matching the backend that will
  execute input/capture.
- This does not certify Windows/Linux product behavior; real hosts and OS-effect
  evidence are still required.

## 2026-08-24 — Target monitor worker ownership must remain acyclic

Decision:

- A background generation worker must not retain the plugin aggregate whose
  destructor owns and joins that worker hierarchy.
- Runtime component lifetime and plugin lifecycle ownership are separate
  responsibilities.

Implementation delta:

- Replaced worker-held `Weak<RemoteDesktopPlugin>` upgrades with a focused
  component context containing session, transport, and recovery stores.
- Added regression coverage that the worker context does not increase the
  plugin aggregate strong count while its components remain alive.

Product effect:

- Plugin shutdown cannot enter the observed supervisor/generation circular
  join when the final aggregate reference is released on a worker thread.
- Live daemon/plugin crash-restart recovery is still required before product
  completion can be claimed.

## 2026-08-24 — Audio transport backpressure must not own session progress

Decision:

- A live audio sample write may wait indefinitely for transport capacity, so it
  cannot execute inside the media control loop.
- Real-time audio freshness has a finite latency budget. Once the encoded audio
  queue is full, the oldest packet is discarded before admitting the newest;
  reconnect recovery must not replay stale audio.
- Moving writes to a worker is insufficient if completion reporting creates an
  unbounded per-packet queue. Writer observations must remain fixed-size.

Implementation delta:

- Added a session-owned, abortable native audio RTP writer.
- Bounded capture and encoded-packet queues to four entries each; the encoded
  queue represents at most 80 ms of pending 20 ms Opus packets.
- Replaced per-packet completion messages with atomic totals plus one terminal
  error slot.
- Added runtime stats, evidence aggregation/verification, frontend projection,
  and regression coverage for transport isolation, queue bounds, stale-packet
  drops, and counter consistency.

Product effect:

- Slow or disconnected audio transport can no longer block video adaptation,
  target rebind, cancellation, statistics, or terminal lifecycle progress.
- Source and contract tests do not prove a live slow-receiver path; the real
  baseline/degraded-network/backpressure media matrix remains required before
  product completion can be claimed.

## 2026-08-24 — retry_session replaces transport, not the session aggregate

Decision:

- `retry_session` is a recovery instruction for an existing non-terminal
  RemoteApp session. It preserves session identity, token, consent, selected
  Resource URA, event history, and authority context.
- Recovery may retire the old PeerConnection and negotiate a strictly newer
  transport epoch. It must not invoke `end_session`, mint another consent grant,
  or call `create_session`.
- `new_session_required` remains the only recovery outcome that authorizes a
  replacement session.

Implementation delta:

- Reused the store-owned device-resume transport state machine for the explicit
  Retry session action and exposed it as `rdRetry`.
- Renamed its in-flight identity guard from resume-specific to retry-specific so
  presence recovery and user retry share one idempotent operation.
- Added a monotonic retry generation in addition to session identity. Offline,
  end, and reset invalidate that generation, and WebRTC negotiation checks both
  operation ownership and PeerConnection ownership after every awaited step.
  This prevents same-session ABA and stale async continuation resurrection.
- Changed UI recovery copy and capability gates to the actual same-session
  abilities: `show_session`, `set_description`, `add_ice_candidate`,
  `watch_events`, and `refresh_lease`.
- Replaced the source gate that required end-then-create with gates proving no
  end/create call, stable session/token, watch reattachment, and a newer epoch.

Product effect:

- A daemon `SESSION_REHYDRATED`, `SESSION_DEGRADED`, or `TRANSPORT_FAILED`
  recovery instruction no longer destroys the aggregate it asks the client to
  recover.
- Component/store tests are not live crash/restart evidence; the real recovery
  runner must still prove post-restart media frames and input on the same
  session.

## 2026-08-24 — recovery retention is one memory-and-disk decision

Decision:

- A terminal RemoteApp row pruned from the canonical in-memory session store
  must be deleted from daemon-local recovery persistence in the same maintenance
  operation. A separate disk-retention policy would create two session truths.
- Snapshot IO is bounded to 4 MiB per row. Reads stop before JSON decode and
  writes stop during serialization, rather than allocating an arbitrary body
  and checking only afterward.
- Commit, load, and delete use one recovery-store lock. Per-session lock files
  would themselves accumulate after tombstone deletion.

Implementation delta:

- `prune_terminal_rows_to_active_bound_locked` now returns the exact removed
  session ids instead of a count.
- `RemoteDesktopSessionPrune` carries retained expiry snapshots and removed ids;
  create-session maintenance persists the first set and deletes the second.
- `RemoteDesktopRecoveryStore::delete` is idempotent, and bounded reader/writer
  adapters reject oversized snapshots before decode or atomic write.
- Startup derives maximum snapshot rows from the canonical active-plus-terminal
  session retention formula, caps every directory entry (including sidecars),
  and enforces a 64 MiB aggregate batch bound before decode.
- Tests cover bounded read/write, idempotent delete, terminal absorption,
  concurrent commit convergence, and durable removal of a pruned terminal row.

Product effect:

- Normal long-running use no longer grows one durable JSON snapshot per
  historical RemoteApp session after the memory aggregate has discarded it.
- Legacy or corrupt stores above row, directory, or byte bounds now fail closed
  with a typed startup error instead of consuming unbounded time or memory.
- Recovery-store bounds do not replace the still-missing live daemon-crash,
  media-reattachment, rendered-frame, and input-after-restart evidence;
  product completion remains false.

# Decisions and Deltas — RemoteApp Product Closure

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

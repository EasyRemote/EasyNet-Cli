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

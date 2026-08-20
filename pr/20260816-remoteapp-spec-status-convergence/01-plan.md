# RemoteApp SPEC status convergence plan

## Intent

Align the RemoteApp targeted-session SPEC status section with the implemented
runtime model without changing the normative architecture direction.

## Boundary

- Do not relax capture, input, target-binding, transport, or E2E acceptance
  requirements.
- Do not claim decoded-frame or browser/backend live E2E completion without
  executing those gates.
- Preserve the SPEC rule that multi-display application capture is either
  `MultiAppSurface` or explicit unsupported; the current implementation remains
  explicit unsupported for multi-display app window sets.
- Preserve the SPEC rule that interactive app/window input remains view-only
  until focus-safe input validation is proven.

## Invariants

- `callee`/service ownership semantics are not changed in this iteration.
- RemoteApp production media still starts only from committed
  `RemoteAppTargetBinding`.
- Configured STUN/TURN/EasyNet relay route support is documented as a provider
  backed implementation, while production relay deployment E2E remains open.
- Same-display application window-set rebind is documented as implemented only
  through the explicit pending-media-rebind state machine and
  `TARGET_REBOUND`/`TARGET_REBIND_FAILED` lifecycle events.

## Verification plan

- Run the RemoteApp lifecycle boundary script and its mutation tests.
- Run targeted Rust tests for route projection and target rebind.
- Run `git diff --check`.
- Keep EasyNet Frontend verification read-only unless frontend sources change.

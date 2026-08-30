# RemoteApp input permission restore slice

## Product gap

After a host runtime reports an OS input-injection permission denial, the
RemoteApp session correctly downgrades from `InputActive` to `MediaActive` and
keeps media alive. The missing closure is the recovery edge: once the operator
restores host input permission and a later input frame is actually applied, the
session must clear the runtime blocker and re-enter `InputActive`.

## Boundary decision

- The WebRTC input plane may observe that an input frame was accepted by the
  host OS.
- The session aggregate owns lifecycle mutation, blocker clearing, and event
  projection.
- The frontend must not infer recovery from `INPUT_FRAME_APPLIED`; it should see
  an explicit session event and a cleared `input_readiness.blocked_reason`.

## Invariants

1. A runtime input permission block never fails active media.
2. A successful input frame for the current transport epoch is sufficient proof
   that the runtime permission blocker has been resolved.
3. Blocker recovery is edge-triggered and projected as
   `INPUT_PERMISSION_RESTORED`.
4. Stale transport epochs and terminal sessions cannot clear the blocker.
5. The data-channel loop does not own lifecycle state; it calls a store boundary
   after input execution succeeds.

## Verification

- Session unit tests prove blocked input reactivates after an applied input
  frame and emits one restore event.
- Session event tests pin the `INPUT_PERMISSION_RESTORED` payload.
- Static lifecycle gates require the input applied path to report the applied
  frame to the session aggregate.

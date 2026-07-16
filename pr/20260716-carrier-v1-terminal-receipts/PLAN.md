# Carrier-v1 Terminal Receipt Plan

## Goal

Make carrier-v1 stream terminality receipt-backed. A `DispatchResult` with
`terminal=true` must always carry `terminal_receipt`; transport, admission,
projection, and pre-terminal control failures must not masquerade as canonical
runtime terminal frames.

## Invariants

- Carrier terminality is lifecycle terminality, not transport completion.
- Stream admission, progress, control failure, and terminal frames are distinct
  phases.
- Any stream `DispatchResult` with `terminal=true` must include exactly one
  terminal receipt and no repeated admission receipt.
- Pre-terminal failures may complete the pending caller with an error, but must
  remain `terminal=false` on the carrier wire.

## Scope

- Tighten carrier-v1 stream result classification.
- Convert local carrier-v1 stream producer synthetic failure frames to
  non-terminal control failures.
- Add focused regression tests around classifier and local producer behavior.

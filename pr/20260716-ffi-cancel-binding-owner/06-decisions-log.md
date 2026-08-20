# Decisions Log

## 2026-07-16

- Chose deletion over `#[allow(dead_code)]` because raw-handle cleanup is an
  obsolete ownership path.
- Kept `cancel_invocations_for_binding` as the only cleanup entry because it
  preserves handle incarnation and matches shutdown's existing state machine.
- Kept public FFI cancellation APIs unchanged; this slice only removes an
  unused internal helper.

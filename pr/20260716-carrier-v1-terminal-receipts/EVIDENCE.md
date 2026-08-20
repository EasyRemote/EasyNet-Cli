# Carrier-v1 Terminal Receipt Evidence

## Source Exploration

- `src/daemon/invocation/dispatch/local_session_dispatcher.rs` documents
  carrier-v1 stream terminal frames as requiring
  `DispatchResult.terminal_receipt`.
- The same producer emits `terminal=true` with only `failure` for stream open
  failure, admission receipt failure, admission projection failure,
  finalization failure, terminal projection failure, and stream exhaustion
  without terminal.
- `src/daemon/invocation/bidi/bidi_dispatcher.rs` currently rejects missing
  terminal receipts only for successful stream terminals; failed stream
  terminals without a receipt are accepted into the terminal settlement path.

## Boundary Decision

The receiving classifier owns carrier protocol validity. The local stream
forwarder owns carrier frame projection. Both must agree that only a verified
runtime terminal receipt can set carrier `terminal=true`.

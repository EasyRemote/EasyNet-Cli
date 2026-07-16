# Carrier-v1 Terminal Receipt Verification

## Commands

- `cargo test carrier_v1_stream --lib`
  - Result: passed.
  - Coverage: stream control failures stay non-terminal, stream terminal
    failures require terminal checkpoints, terminal stream frames carry receipt
    checkpoints, and admission/terminal checkpoint geometry remains separated.
- `bash tools/scripts/check-architecture-convergence.sh`
  - Result: passed.
- `bash tools/scripts/check-project-structure-v1.sh`
  - Result: passed.
- `git diff --check -- src/daemon/invocation/dispatch/local_session_dispatcher.rs src/daemon/invocation/bidi/bidi_dispatcher.rs`
  - Result: passed.

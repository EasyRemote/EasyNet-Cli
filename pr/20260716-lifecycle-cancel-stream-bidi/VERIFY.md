# Stream/Bidi Lifecycle Cancellation Verification

## Commands

- `cargo test carrier_v1_ --lib`
  - Result: passed.
  - Coverage: carrier-v1 unary/stream/bidi classifier and local session
    dispatch tests, including stream and bidi lifecycle registry assertions.
- `cargo test invocation_cancel --lib`
  - Result: passed.
  - Coverage: signed invocation cancel replay rejection and real handler
    routing.
- `bash tools/scripts/check-architecture-convergence.sh`
  - Result: passed.
- `rm -rf sdk/conformance/__pycache__ && bash tools/scripts/check-project-structure-v1.sh && test ! -d sdk/conformance/__pycache__`
  - Result: passed.
- `git diff --check -- src/daemon/invocation/dispatch/local_session_dispatcher.rs src/daemon/invocation/dispatch/cancellation.rs src/daemon/ability/builtins/governance/invocation_cancel.rs`
  - Result: passed.

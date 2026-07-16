# Verification Plan

## Focused Behavior

- Add a service-level regression test where a registered local stream ability
  emits one progress frame, the test drops the response stream, and the ability
  observes Axon cancellation.

## Commands

```bash
cargo test invoke_stream_cancels_local_runtime_when_client_drops_response --lib
cargo test invoke_stream --lib
bash tools/scripts/check-architecture-convergence.sh
rm -rf sdk/conformance/__pycache__
bash tools/scripts/check-project-structure-v1.sh
test ! -d sdk/conformance/__pycache__
git diff --check -- src/daemon/invocation/streams/stream_dispatcher.rs src/daemon/invocation/dispatch/daemon_invocation_service_tests/stream.rs pr/20260716-local-stream-consumer-cancel
```

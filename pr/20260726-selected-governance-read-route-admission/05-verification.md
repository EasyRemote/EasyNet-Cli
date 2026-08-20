Verification matrix
===================

Executed checks:

- `cargo test dispatch_local_rpc_rejects_receipt_history_before_local_runtime_admission --lib` — passed.
- `cargo test dispatch_remote_rpc_rejects_receipt_history_as_public_remote_action --lib` — passed.
- `cargo test dispatch_remote_rpc_allows_receipt_history_with_resource_read_subject --lib` — passed.
- `cargo test dispatch_remote_rpc_rejects_catalogue_read_with_public_action_subject --lib` — passed.
- `cargo test dispatch_remote_rpc_allows_catalogue_read_with_runtime_read_subject --lib` — passed.
- `cargo test invoke_stream_rejects_governance_catalogue_route_before_presence_forwarding --lib` — passed.
- `cargo test remote_bidi_rejects_governance_catalogue_route_before_carrier_frame --lib` — passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `bash tools/scripts/check-architecture-convergence.sh` — passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.

Additional observation:

- `cargo test daemon::invocation::dispatch::daemon_invocation_service::tests::unary --lib`
  still has two unrelated federation join fixture failures where the test emits a
  non-URA caller. The selected governance-read tests in that module passed.

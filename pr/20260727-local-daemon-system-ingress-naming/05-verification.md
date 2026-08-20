# Verification

## Completed

- `cargo test local_system_tuple_plan --lib` — passed 3 tests.
- `cargo test local_system_invoke_request --lib` — passed 1 test.
- `cargo test dispatch_local_rpc_selected_route_accepts_unsigned_local_system_request --lib` — passed 1 test.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `bash tools/scripts/check-architecture-convergence.sh` — passed.
- `bash tools/scripts/check-daemon-invocation-migration.sh` — passed.
- `bash tools/scripts/check-sdk-product-neutrality.sh` — passed.
- `bash tests/scripts/test_check_daemon_invocation_migration.sh` — passed.
- `codegraph sync .` and query for `LocalDaemonSystemInvocation LocalDaemonLoopbackInvocation local_daemon_loopback` — confirmed the active source graph resolves `LocalDaemonSystemInvocation`.

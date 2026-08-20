# Verification

## Checks

- `cargo test --lib invoke_dispatches_federation_discover_includes_local_presence_devices -- --nocapture`
- `cargo test --lib federation_discover_subject -- --nocapture`
- `cargo test --lib local_daemon_ura -- --nocapture`
- `rustfmt --edition 2021 --check src/daemon/identity/local_invocation.rs src/daemon/invocation/routing/remote_invoke.rs src/daemon/invocation/dispatch/unary_dispatcher.rs src/daemon/invocation/dispatch/daemon_invocation_service_tests/unary.rs src/support/platform/local_daemon_grpc.rs`
- `git diff --check -- src/daemon/identity/local_invocation.rs src/daemon/invocation/routing/remote_invoke.rs src/daemon/invocation/dispatch/unary_dispatcher.rs src/daemon/invocation/dispatch/daemon_invocation_service_tests/unary.rs src/support/platform/local_daemon_grpc.rs pr/20260716-federation-discover-daemon-identity-presence`

## Boundary Evidence

- CodeGraph path: `invoke_federation_discover_filtered -> local_daemon_ura -> local_device_ura`.
- CodeGraph blast radius: unary discover dispatcher, CLI discover helper, and
  local daemon default callee.

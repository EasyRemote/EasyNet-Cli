# Local Loopback Owner Gate

## Root Fork

The local daemon loopback request shape was moved from
`support/platform/local_daemon_grpc.rs` into
`daemon::invocation::dispatch::invocation_wire`, but without an executable
architecture gate the same fork can return silently: support code could define
another loopback request object or call `ProtoEnvelope::targeted` directly.

## Expected Effect

Architecture convergence. The previous ownership migration becomes durable:
`support/platform` stays a transport adapter and the daemon Invocation wire
module remains the sole owner of local loopback protobuf request construction.

## Boundary Invariants

1. `LocalDaemonLoopbackInvocation` is defined only in
   `src/daemon/invocation/dispatch/invocation_wire.rs`.
2. `src/support/platform/local_daemon_grpc.rs` may import and call the
   daemon-owned object, but must not define its own loopback request struct.
3. The support adapter must not call `ProtoEnvelope::targeted` directly.
4. The daemon-owned loopback object must retain unary request, stream request,
   envelope, causal-context, trace-id, function-name, caller, and argument
   projections.

## Verification

- Add `check-architecture-convergence.sh` rule coverage for the local loopback
  request owner.
- Add negative fixture tests so the checker fails if the support adapter
  reintroduces a local struct or direct `ProtoEnvelope::targeted` call.
- Run the architecture checker and its script tests.

Commands run:

- `bash -n tools/scripts/check-architecture-convergence.sh`
- `bash -n tests/scripts/test_check_architecture_convergence.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`

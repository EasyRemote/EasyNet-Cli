# Local Loopback Invocation Wire Boundary

## Root Fork

`src/support/platform/local_daemon_grpc.rs` owns local socket transport, but it
also keeps a `LocalDaemonLoopbackInvocation` value that stores caller, callee,
subject, arguments, causal context, trace id, and timeout before manufacturing
Axon `InvokeRequest` and `InvokeServerStreamRequest` protobufs.

That duplicates the daemon Invocation module's stated ownership of outbound
wire request construction and lets a support adapter become a second source for
Invocation envelope shape.

## Expected Effect

Effect convergence. The support adapter should keep only socket probing,
tonic transport, and JSON projection. The daemon Invocation wire boundary
should own the inspectable request value that turns a seven-field local
loopback tuple into protobuf request shapes.

This slice does not add a product capability and does not change public CLI
behavior.

## Boundary Invariants

1. The local loopback caller remains the daemon local-system Agent URA.
2. The request keeps `function_name` as the route query and must not pre-bind
   descriptor metadata.
3. The envelope still carries explicit caller, callee, subject, nonce, causal
   context, and optional trace id before transport dispatch.
4. `support/platform` must not store or assemble Axon envelope identity fields
   in its own invocation object after migration.
5. The daemon Invocation wire module remains a protobuf construction boundary;
   Axon still owns canonical descriptor-bound bytes, admission, signing, and
   receipt verification.

## Migration Plan

1. Add a daemon-owned `LocalDaemonLoopbackInvocation` next to `ProtoEnvelope`.
2. Move request, stream request, envelope, causal context, trace id, and timeout
   projection into that object.
3. Replace the support-layer struct with calls into the daemon-owned object.
4. Keep target and subject policy resolution in support for now because it is
   local CLI transport policy, not Axon envelope construction.
5. Verify the existing loopback test still proves descriptor metadata is not
   pre-bound.

## Deletion Condition

After this slice, there must be no support-layer `LocalDaemonLoopbackInvocation`
struct that mirrors Axon envelope fields. Any future loopback request shape
changes should happen in `daemon::invocation::dispatch::invocation_wire`.

## Verification

- `rg -n "struct LocalDaemonLoopbackInvocation|LocalDaemonLoopbackInvocation::from_subject_policy" src/support/platform/local_daemon_grpc.rs src/daemon/invocation/dispatch/invocation_wire.rs`
  confirms the struct now exists only in the daemon Invocation wire module and
  the support-layer constructor path is gone.
- `rustfmt --edition 2021 --check src/daemon/invocation/dispatch/invocation_wire.rs src/support/platform/local_daemon_grpc.rs`
- `cargo test -p easynet loopback_invoke_request_does_not_pre_resolve_descriptor_ref --features axon-pb -- --nocapture`
- `cargo check -p easynet --features axon-pb`
- `bash tools/scripts/check-invocation-unity.sh`
- `bash tools/scripts/check-architecture-convergence.sh`

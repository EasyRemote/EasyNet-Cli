# Architecture

## Layering

- `src/daemon/identity/local_invocation.rs` owns daemon identity discovery.
- `src/support/platform/local_invoke.rs` owns public local ability helper API.
- `src/support/platform/local_daemon_grpc.rs` owns protobuf loopback transport
  and tuple construction.

## Boundary change

Before this task, the transport tuple plan supported a `LocalDaemonSelf` subject
policy that resolved subject from callee. That mixed target selection and
subject selection inside the transport adapter.

After this task, the generic local ability helper obtains the daemon subject
from the identity boundary first, then passes it through the same explicit
subject policy used by other daemon-system loopback calls.

## Ownership

The transport adapter no longer owns a subject derivation rule. It only validates
and carries an explicit subject supplied by the issuer.

# Architecture

`src/support/platform/local_daemon_grpc.rs` is a CLI/platform helper that submits complete invocation tuples to the local daemon.

The existing `Loopback` naming is misleading because the important semantic is not socket topology; it is daemon-local system authority. The helper already sends `_system.local` caller facts and the daemon dispatch layer upgrades eligible requests through `TrustedLocalSystem`.

The new naming keeps transport details separate:

- `LocalDaemonSystemTuplePlan`
- `LocalDaemonSystemInvocation`
- local-system caller helper functions

Actual TCP loopback references in networking tests remain unchanged.

# Architecture

## Ownership

This slice is documentation convergence for already-landed daemon ownership:

- `src/daemon/control/` owns boot/status control frames.
- `src/daemon/invocation/` owns daemon Invocation ingress.
- `easynet_axon::invocation::LocalRuntime` is embedded by the daemon for
  admitted local execution.
- `src/daemon/boot/join_connection_state.rs` owns join/status state snapshots.

## Retired Surfaces

The previous `src/daemon/control/runtime_dispatch*.rs` callback-socket surface
is absent from current source and should not be described as an active boundary.
Historical docs may mention it only as history or audit evidence.

## Boundary

The docs must keep process lifecycle and product policy in EasyNet-Cli while
leaving Axon as protocol/runtime primitive owner.

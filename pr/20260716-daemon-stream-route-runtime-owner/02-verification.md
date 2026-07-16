# Verification

## Planned checks

- `rustfmt --edition 2021 --check src/daemon/invocation/dispatch/daemon_invocation_service.rs src/daemon/invocation/dispatch/daemon_route_runtime.rs src/daemon/invocation/streams/stream_dispatcher.rs`
- `cargo check --features axon-pb`
- Focused stream exact route tests.
- Exact stream lifecycle-drop closure test.
- `tools/scripts/check-architecture-convergence.sh`
- `tests/scripts/test_check_architecture_convergence.sh`
- Scoped `git diff --check`

## Runtime-owner proof

- Exact stream routes are registered in `LocalRuntime` before listeners start.
- `InvokeStream` exact-route matches delegate to `StreamDispatcher::dispatch_daemon_route_runtime`.
- `DaemonRouteRuntimeAdapter::open_stream` builds descriptor-bound wire requests and calls Axon's admitted stream path.
- Product directory stream logic is isolated behind `DaemonStreamRouteProvider` and returns `StreamSource`; it does not construct `InvokeStreamChunk`, admission receipts, or terminal receipts.
- `DaemonStreamRouteProvider` holds weak lifecycle and presence references, so dropping `DaemonInvocationService` terminates directory pumps instead of leaving product stream tasks detached from their runtime owner.

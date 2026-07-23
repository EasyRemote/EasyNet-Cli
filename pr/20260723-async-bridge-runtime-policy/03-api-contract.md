# API Contract

## Internal Rust API

- `SyncBridgeRuntimePolicy::UseFuturesExecutor`
- `SyncBridgeRuntimePolicy::BuildCurrentThreadTokio`
- `run_blocking(future, policy)`
- `try_run_blocking(future, policy, bridge_label)`

## Public behavior

No user-visible output, wire shape, SDK API, or product command behavior changes.

## Error behavior

`try_run_blocking` still returns `Err(String)` with the bridge label when helper
runtime construction fails.

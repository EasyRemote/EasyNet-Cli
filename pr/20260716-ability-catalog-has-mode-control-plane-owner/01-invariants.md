# Invariants

- `AbilityControlPlaneRegistry` is the canonical metadata/proof store.
- `ExecutionIndex` is the only catalog handler-presence index.
- Public routeability checks require both a control-plane mode record and an
  execution-index handler for that mode.
- RPC-name publication is projected from committed RPC control-plane records.
- `LocalRuntime` is not queried by `has_rpc`, `has_stream`, or `has_bidi`.
- A broken/missing control-plane row must not be masked by a handler entry or
  runtime ability option.
- Runtime ability option checks remain valid in tests that explicitly verify
  runtime installation, but not as catalog presence fallback.

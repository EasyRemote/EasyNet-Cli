# Invariants

- Every submitted invocation visible to SDK clients has a deterministic terminal projection.
- Receipt-bearing terminal outcomes remain the normal successful or post-admission failure path.
- Receipt-free outcomes are fail-closed and limited to pre-admission failures with explicit `Failed` lifecycle state and typed error facts.
- Go and Python SDKs consume the same C ABI outcome contract.
- No product-specific EasyNet/EasyRemote error alias is restored in the canonical SDK.

SDK runtime identity explicit endpoint cutover

## Objective

Close the remaining A49 runtime-identity alias surface by deleting the Go SDK
`DefaultRuntimeIdentitySocketPath` compatibility function and the Python SDK
`default_runtime_keyring_socket_path` compatibility function. Runtime signing
identity must be reached through an explicit daemon key-service endpoint carried
by `RuntimeSigningIdentityRequest`, `EnsureRuntimeSigningIdentityRequest`, or
the Python `socket_path` arguments.

## Expected effect

| Dimension | Expected convergence |
|---|---|
| Architecture convergence | Runtime identity has one SDK entry path in Go and Python: caller supplies the product-owned daemon key-service endpoint explicitly. |
| Architecture cleanliness | Remove public functions whose only behavior was an error for legacy source compatibility. |
| Product acceleration | Product runtimes can wire their lifecycle endpoint directly without a misleading default-discovery API in the canonical SDK. |
| Risk | This is a deliberate public inventory contraction for unusable compatibility symbols; usable runtime identity behavior is unchanged. |

## Non-goals

- Do not add SDK-owned EasyNet directory discovery.
- Do not introduce fallback endpoint search.
- Do not change daemon key-service protocol fields or signing behavior.
- Do not touch unrelated dirty documentation and skill files.

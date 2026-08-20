# Architecture

## Boundary

`src/ffi/invocation/mod.rs` owns the C ABI projection and request decoding.
It may convert typed internal failures to ABI error projections, but it must not
infer runtime state from daemon error message strings.

## Refactor shape

The remote descriptor probe remains a small state object. Its `prepare` phase
owns caller signer lookup and its `invoke` phase owns one remote
`meta.list_abilities` request. Both phases return `DescriptorResolutionError`
directly.

## Removed coupling

The FFI layer no longer depends on daemon text fragments such as
`NEGATIVE_REASON_NXDOMAIN` or `owner is not online` for canonical error codes.

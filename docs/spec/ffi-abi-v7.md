# EasyNet Generic C ABI v7

Status: current release contract with feature-detected v8 raw-stream and v9
buffer-lease extensions.

The v7 ABI is a capability-neutral runtime boundary. It exposes daemon
lifecycle, complete generic Invocation lifecycle, stream/bidi control, and
stable runtime/error JSON DTOs. Authority, Identity, Directory, Receipt,
Publication, Host Binding, Mission, Events, Admin/Gateway, Surface,
Compatibility, Wrapper, and companion helpers are language-SDK concerns and
must not be exported from `libeasynet_cli`.

## Canonical surface

- Header: `include/easynet_cli.h`
- Exact base export allowlist: `include/easynet_cli.exports.v7`
- Exact raw-stream extension allowlist: `include/easynet_cli.exports.v8`
- Exact buffer-lease extension allowlist: `include/easynet_cli.exports.v9`
- ABI version: `7`
- Base export count: exactly `56`
- v8 extension export count: exactly `57`
- v9 extension export count: exactly `60`

Release and CI checks compare both header declarations and normalized dynamic
library exports against the allowlists. Missing and unexpected symbols are ABI
failures. The v8 allowlist must include every v7 symbol and add only
`runtime_invocation_stream_open_v8`; `runtime_abi_version()` remains `7` and
bindings must use `runtime_feature_discovery` before calling the v8 entry
point. Earlier ABI documentation is historical and is not a release input.

The v9 allowlist includes every v8 symbol and adds only
`runtime_invocation_stream_open_v9`, `runtime_buffer_lease_retain_v9`, and
`runtime_buffer_lease_release_v9`. Its ownership contract is specified by
`docs/spec/ffi-abi-v9.md`; it does not change `runtime_abi_version()` or v8.

## Ownership state machines

- `runtime_init` returns an `RuntimeHandle`; `runtime_shutdown` is its terminal
  release operation.
- daemon start/attach return `RuntimeHostHandle`; stop/detach are terminal.
- builder new returns `RuntimeInvocationBuilderId`; build/prepare consume on
  success, while builder free is the explicit release path.
- prepare returns `RuntimePreparedInvocationId`; signing consumes it on
  success, while prepared free is the explicit release path.
- signing returns `RuntimeSignedInvocationId`; submit-handle consumes it on
  success, while signed free is the explicit release path.
- submit-handle returns `RuntimeInvocationHandleId`; await/cancel/events observe
  it and handle-free releases it.
- stream cancel and bidi cancel are cancel-request operations at this provider
  boundary. Each registered resource submits at most one independently signed
  canonical `invocation.cancel` command, memoizes its acceptance or rejection,
  and keeps the callback/reader path draining. A duplicate request observes the
  memoized result and never submits another command. Cancellation must not claim
  lifecycle terminality without the original invocation's canonical terminal
  receipt.
- stream close and bidi close are local resource release operations. Bidi
  close-send is a non-terminal local half-close.

Every returned `char *` is caller-owned and must be released exactly once with
`runtime_string_free`. Callback JSON pointers are borrowed only for the callback
duration.

## Errors and capability discovery

All fallible operations return an integer error code. Bindings use
`runtime_last_error_json` or `runtime_error_json`; the legacy borrowed
`runtime_last_error` pointer is not part of v7.

`runtime_feature_discovery` advertises only the `runtime_core` C profile and
generic runtime symbols. Language SDK capability state is maintained separately
in `sdk/conformance/sdk-parity-matrix.json`; an absent high-level provider must
produce an explicit typed `NotImplemented` result and must never trigger a
product-symbol lookup or compatibility fallback.

# EasyNet Generic C ABI v5

Status: current release contract.

The v5 ABI is a capability-neutral runtime boundary. It exposes daemon
lifecycle, complete generic Invocation lifecycle, stream/bidi control, and
stable runtime/error JSON DTOs. Authority, Identity, Directory, Receipt,
Publication, Host Binding, Mission, Events, Admin/Gateway, Surface,
Compatibility, Wrapper, and companion helpers are language-SDK concerns and
must not be exported from `libeasynet_cli`.

## Canonical surface

- Header: `include/easynet_cli.h`
- Exact export allowlist: `include/easynet_cli.exports.v5`
- ABI version: `5`
- Export count: exactly `54`

Release and CI checks compare both header declarations and normalized dynamic
library exports against the allowlist. Missing and unexpected symbols are ABI
failures. Earlier ABI documentation is historical and is not a release input.

## Ownership state machines

- `easynet_init` returns an `EasynetHandle`; `easynet_shutdown` is its terminal
  release operation.
- daemon start/attach return `EasynetDaemonHandle`; stop/detach are terminal.
- builder new returns `EasynetInvocationBuilderId`; build/prepare consume on
  success, while builder free is the explicit release path.
- prepare returns `EasynetPreparedInvocationId`; signing consumes it on
  success, while prepared free is the explicit release path.
- signing returns `EasynetSignedInvocationId`; submit-handle consumes it on
  success, while signed free is the explicit release path.
- submit-handle returns `EasynetInvocationHandleId`; await/cancel/events observe
  it and handle-free releases it.
- stream cancel and bidi cancel are cancel-request operations at this provider
  boundary; they release local callback/reader resources and must not claim
  lifecycle terminality without a canonical terminal receipt.
- stream close and bidi close are local resource release operations. Bidi
  close-send is a non-terminal local half-close.

Every returned `char *` is caller-owned and must be released exactly once with
`easynet_string_free`. Callback JSON pointers are borrowed only for the callback
duration.

## Errors and capability discovery

All fallible operations return an integer error code. Bindings use
`easynet_last_error_json` or `easynet_error_json`; the legacy borrowed
`easynet_last_error` pointer is not part of v5.

`easynet_feature_discovery` advertises only the `runtime_core` C profile and
generic runtime symbols. Language SDK capability state is maintained separately
in `sdk/conformance/sdk-parity-matrix.json`; an absent high-level provider must
produce an explicit typed `NotImplemented` result and must never trigger a
product-symbol lookup or compatibility fallback.

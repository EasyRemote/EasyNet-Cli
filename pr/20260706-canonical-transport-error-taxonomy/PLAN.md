# Canonical Transport Error Taxonomy Plan

## Goal

Converge SDK-created transport failures onto the SPEC section 22 canonical
RuntimeError code `ROUTE_UNAVAILABLE` while preserving legacy `TRANSPORT` only
as a compatibility input alias.

## Root Problem

The previous error-code slice normalized daemon/C ABI wire aliases, but Go and
Python still constructed new facade errors with the retired `TRANSPORT` code.
That lets old vocabulary leak from profile lifecycle, runtime transport,
stream/bidi, and C ABI dynamic loading paths.

## Boundary Proof

- Public legacy constants remain available for source compatibility.
- `NormalizeErrorCode` / `normalize_error_code` continue to accept `TRANSPORT`.
- Newly constructed SDK transport failures use `ROUTE_UNAVAILABLE`.
- Retry semantics remain `safe` unless an existing path deliberately uses a
  narrower hint.
- No SPEC edits.

## Implementation Order

1. Add/use canonical transport error helpers at the shared error boundary.
2. Migrate Go runtime/profile transport helpers from `ErrTransport` to
   `ErrRouteUnavailable`.
3. Migrate Python SDK transport helper constructors from `ErrorCode.TRANSPORT`
   to `ErrorCode.ROUTE_UNAVAILABLE`.
4. Update tests to assert canonical construction while keeping legacy alias
   normalization tests.
5. Run focused Go/Python error and runtime gates, scaffold, and conformance.

## Verification

- `gofmt`
- `python -m compileall sdk/python/easynet_sdk`
- `(cd sdk/go && go test ./...)`
- `(cd sdk/python && uv run python -m unittest discover tests)`
- `cargo test --bin sdk-conformance-runner`
- `bash tools/scripts/check-sdk-scaffold.sh`
- Four-language adapter reports through `sdk-conformance-runner` for Rust,
  C ABI, Go, and Python.
- `git diff --check`

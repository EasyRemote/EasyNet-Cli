# Go Control Discovery Runtime Connector

## Intent

Move Go toward backend cutover by giving the Go SDK the same process-root
connection shape as Python: `control.json` resolves the daemon invocation
endpoint, and `RuntimeConnection` owns the resolve -> handshake -> ready state
transition. This slice deliberately does not import Axon or generated protobufs.

## Boundary Proof

- Go SDK public package must not import Axon, generated Axon protobufs, cgo, or
  daemon internals.
- `control.json` parsing is daemon discovery projection, not Invocation wire
  semantics.
- The connector owns endpoint resolution only. Handshake and Invocation dispatch
  stay delegated to an inner `RuntimeConnector`.
- Missing invocation endpoints are surfaced as `ErrControlOnly`, not silently
  defaulted.

## Verification

- `go test ./...` in `sdk/go`.
- Existing Go import-boundary test continues to ban Axon/protobuf imports.
- `bash tools/scripts/check-sdk-parity-matrix.sh --self-test`.
- `bash tools/scripts/check-sdk-scaffold.sh`.

## Remaining After This Slice

- Concrete Go direct daemon UDS unary/stream/bidi transport without Axon/protobuf
  imports.
- live daemon keyring signing execution policy.
- RFC-007 receipt URA construction.
- EasyRemote repository extraction/cutover gate.
- EasyNet backend SDK-only import/route cutover gate.
- daemon-side Mission child Invocation execution and scheduler/retry policy.

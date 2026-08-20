# Go Receipt Causal Ref Hash Closure

## Objective

Align the Go Receipt facade with the daemon/Axon and Python Receipt projection
contract for child-invocation causal refs. A causal ref is usable only when it
carries an explicit receipt URA plus a 32-byte receipt hash in the
`causal_context` projection.

## Boundary

- Axon remains the verifier and protocol owner.
- EasyNet-Cli/C ABI remains the projection provider.
- Go SDK parses and preserves the provider projection; it does not construct
  receipt URAs or verify receipts locally.

## Invariants

1. `CausalRef` decoding fails closed when `receipt_ura` is absent.
2. `CausalRef` decoding fails closed when `receipt_hash_hex` or equivalent
   receipt hash input is absent.
3. If `causal_context` is present, the hash must be inside that context; Go must
   not repair a weak context from top-level fields.
4. The public child invocation context is returned as a defensive copy.
5. C ABI fake fixtures mirror the real C ABI causal-ref projection shape.

## Verification

- `go test ./... -run 'Receipt|Conformance'` in `sdk/go`
- `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json`
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `bash tools/scripts/check-sdk-scaffold.sh`

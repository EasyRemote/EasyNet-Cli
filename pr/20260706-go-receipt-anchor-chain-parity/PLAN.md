# Go Receipt Anchor Chain Parity

## Objective

Bring the Go Receipt facade to the same SDK-level receipt anchor model already
available in Python: `ReceiptRef` for opaque daemon/Axon receipt anchors and
`ReceiptChain` for ordered continuity checks. The facade may validate shape and
delegate projections, but it must not construct receipt URAs or define Axon
verification semantics locally.

## Boundary Proof

- Axon owns receipt canonicalization, receipt URA semantics, signature
  verification, and causal-context protocol truth.
- EasyNet-Cli daemon/C ABI owns provider projections for receipt verification,
  causal refs, and chain continuity.
- Go SDK owns typed facade objects, validation of required anchor fields, copy
  safety, and delegation to `ReceiptClient`.

## Invariants

- `ReceiptRef` requires an explicit `receipt_ura` and a 32-byte receipt hash
  supplied by daemon/Axon output.
- Hash normalization accepts provider wire aliases such as `self_hash_hex` and
  `sha256:` prefixes but never treats hashes alone as verification evidence.
- `ReceiptChain` preserves order, rejects empty chains, and delegates continuity
  checks through `ReceiptClient.VerifyChain`.
- Child causal context is obtained through `ReceiptClient.CausalRef`; the Go
  facade does not synthesize Axon causal-context semantics.
- All returned maps/slices are copy-safe so callers cannot mutate SDK-owned
  receipt state accidentally.

## Implementation Plan

1. Review the existing uncommitted Go `ReceiptRef`/`ReceiptChain` work for
   compile safety and semantic fit.
2. Add focused Go tests covering JSON projection, runtime-result anchoring,
   chain delegation, invalid hashes, and copy safety.
3. Run receipt-focused Go tests, SDK scaffold/parity gates, conformance runner
   gates for Go/Python, and diff hygiene.
4. Commit the coherent slice with the requested author identity if all gates
   pass.

## Remaining Outside This Slice

- RFC-007 receipt URA construction remains outside SDK implementation until the
  protocol source is finalized.
- Product repository cutovers still need their own import-boundary and route
  smoke evidence before any `cutover-ready` claim.
- Non-P0 language facades remain post-P0 work.

## Verification Results

- `go test ./... -run 'Receipt|Conformance'` passed in `sdk/go`.
- `PYTHONPATH=tests ./.venv/bin/python -m unittest tests.test_receipt tests.test_conformance`
  passed in `sdk/python`.
- `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json --format jsonl`
  passed.
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json --format jsonl`
  passed.
- `cargo run --bin sdk-conformance-runner -- --language c_abi --adapter-report sdk/conformance/runner/c-abi-action-adapter-report.json --format jsonl`
  passed.
- `cargo run --bin sdk-conformance-runner -- --language rust --adapter-report sdk/conformance/runner/rust-action-adapter-report.json --format jsonl`
  passed.
- `tools/scripts/check-sdk-parity-matrix.sh --self-test` passed.
- `bash tools/scripts/check-sdk-scaffold.sh` passed after registering the new
  receipt-ref schema and fixture in the closed scaffold lists.
- `git diff --check` passed.

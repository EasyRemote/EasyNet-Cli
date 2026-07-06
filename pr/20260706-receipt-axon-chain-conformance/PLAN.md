# Receipt Axon Chain Conformance Plan

## Objective

Align the machine-readable SDK parity/conformance ledger with the current
Receipt profile implementation: Axon-backed single-receipt verification and
single-invocation receipt-chain verification with parent-receipt closure are now
provider-backed, while cross-invocation causal DAG verification and RFC-007
receipt URA construction remain explicit remaining work.

## Invariants

- The SPEC remains unchanged.
- No SDK or language facade reimplements Axon receipt semantics.
- Go/Python facades only parse and expose daemon/C ABI projection results.
- Cross-invocation causal DAG verification remains Axon-owned until Axon exposes
  a stable library verifier API instead of only the `easynet-verify` binary.
- Receipt URAs remain opaque daemon/Axon-returned strings while RFC-007 is still
  proposed.

## Implementation Steps

1. Add a shared conformance case for Axon-backed receipt chain verification.
2. Update the SDK parity matrix receipt row to include the new case and precise
   remaining-work wording.
3. Update Go/Python facade tests to pin the parent-closure chain projection
   shape without claiming that language facades perform verification locally.
4. Run matrix self-test plus targeted Go/Python/Rust receipt checks and hygiene.

## Boundary Proof

This is not a new verifier. The verifier implementation already lives in
Rust/C ABI provider code and delegates canonical bytes/signatures to Axon. This
change makes conformance and parity evidence reflect that provider-backed
boundary so downstream cutover decisions stop treating completed single-chain
verification as an open gap.

## Verification Plan

- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `go test ./... -run Receipt`
- `PYTHONPATH=tests uv run python -m unittest tests.test_receipt tests.test_conformance`
- `cargo test receipt_contract --lib`
- `cargo fmt --check`
- `git diff --check`
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`

## Verification Result

- PASS: `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- PASS: `go test ./... -run 'Receipt|ParityMatrix'`
- PASS: `PYTHONPATH=tests uv run python -m unittest tests.test_receipt tests.test_conformance`
- PASS: `cargo test receipt_contract --lib`
- PASS: `cargo fmt --check`
- PASS: `git diff --check`
- PASS: `git diff -- docs/spec/daemon-sdk-requirements-v1.md` produced no diff.
- NOTE: Existing stream/bidi backpressure worktree changes are intentionally
  left outside this Receipt conformance commit and must be handled as a
  separate capability.

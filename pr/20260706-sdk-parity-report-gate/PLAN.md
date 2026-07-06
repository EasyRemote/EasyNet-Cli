# SDK Parity Report Gate Plan

## Intent

Close the gap between Go/Python `provider-backed` parity claims and the
language action-adapter reports that external tooling reads as conformance
evidence.

## Boundary Proof

- Axon remains the owner of URA, DescriptorRef, Invocation, and receipt
  semantics.
- EasyNet-Cli SDK remains a facade/projection over daemon/C ABI providers.
- This change does not add protocol behavior or alter SPEC text. It tightens
  the SDK conformance evidence ledger so Go/Python parity cannot be claimed
  from file existence alone.

## Invariants

1. A `provider-backed` language status must have a passed action-adapter report
   record for every shared case that declares that language in `required_for`.
2. The parity validator must not require a language report for cases that do not
   declare that language.
3. Missing report records must fail the self-test with a specific diagnostic.
4. Existing dirty SPEC or daemon runtime files are outside this task and must
   not be staged.

## Implementation

1. Teach `check-sdk-parity-matrix.sh` to load Go/Python action-adapter reports.
2. Resolve each matrix `shared_cases` file to its case id and `required_for`
   languages.
3. Require `status: passed` report records for `provider-backed` language
   states when the case is required for that language.
4. Register the existing Go/Python receipt chain conformance tests in their
   action-adapter reports.
5. Update the shared parity case expectations and language conformance tests to
   name the stronger evidence gate.

## Verification

- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json`
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json`
- `go test ./... -run 'Conformance|Receipt'` in `sdk/go`
- Python conformance/receipt focused tests in `sdk/python`


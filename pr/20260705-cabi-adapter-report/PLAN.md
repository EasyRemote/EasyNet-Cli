# C ABI Adapter Report Plan

## Goal

Promote the C ABI from manifest-only conformance to the same action-adapter
report gate used by Go and Python for P0 SDK parity.

## Boundary Proof

- SDK-owned:
  - C ABI conformance report metadata.
  - Shared runner validation for required C ABI cases.
  - Scaffold and documentation that make the C ABI report mandatory.
- Product-owned:
  - Backend/EasyRemote product cutover and downstream repository deletion gates.
  - Future Node/JVM/Swift action adapters.

## Invariants

1. The SPEC remains unchanged.
2. The report must be closed over every case that declares `c_abi` in
   `required_for`.
3. Evidence must be repository-local and use `c_abi_test`.
4. The runner must fail if any required C ABI case is missing from the report.
5. No C ABI report record may target a case not declared for `c_abi`.

## Implementation Steps

1. Add `sdk/conformance/runner/c-abi-action-adapter-report.json`.
2. Add the report to scaffold JSON/file guards.
3. Document C ABI adapter-report usage in the conformance suite and runner
   README.
4. Extend runner tests to validate the repository C ABI report.

## Verification

- `cargo test --bin sdk-conformance-runner`
- `cargo run --bin sdk-conformance-runner -- --language c_abi --adapter-report sdk/conformance/runner/c-abi-action-adapter-report.json --format json`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `git diff --check`

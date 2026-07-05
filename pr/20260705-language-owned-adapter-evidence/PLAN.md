# Language-Owned Adapter Evidence Plan

## Goal

Make action-adapter reports prove language-owned evidence instead of accepting
any known test evidence kind for any language.

## Boundary Proof

- SDK-owned:
  - Shared conformance runner.
  - Adapter report validation semantics.
  - Repository-local evidence path validation.
- Product-owned:
  - Product cutover smokes and downstream repository deletion gates.
  - Future non-P0 language evidence manifests.

## Invariants

1. The SPEC remains unchanged.
2. A report for `go` must use `go_test`; `python` must use `python_test`;
   `rust` must use `rust_test`; `c_abi` must use `c_abi_test`.
3. A mismatched evidence kind must fail report loading before action records can
   be counted as passed.
4. Evidence paths must remain repository-local.
5. Existing P0 reports must continue to pass unchanged.

## Implementation Steps

1. Thread the requested language into adapter evidence validation.
2. Derive the expected evidence kind from the report language.
3. Add a regression test for mismatched language evidence.
4. Re-run P0 adapter gates and scaffold checks.

## Verification

- `cargo test --bin sdk-conformance-runner`
- `cargo run --bin sdk-conformance-runner -- --language rust --adapter-report sdk/conformance/runner/rust-action-adapter-report.json --format json`
- `cargo run --bin sdk-conformance-runner -- --language c_abi --adapter-report sdk/conformance/runner/c-abi-action-adapter-report.json --format json`
- `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json --format json`
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json --format json`
- `bash tools/scripts/check-sdk-scaffold.sh`

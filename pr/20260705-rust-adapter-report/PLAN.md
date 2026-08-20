# Rust Adapter Report Plan

## Goal

Promote Rust source-of-truth SDK conformance from manifest-only validation to a
repository adapter report, closing the P0 Rust/C ABI/Go/Python report set.

## Boundary Proof

- SDK-owned:
  - Rust daemon SDK conformance report metadata.
  - Shared runner validation for required Rust cases.
  - Documentation and scaffold guards that make all P0 adapter reports explicit.
- Product-owned:
  - Downstream backend and EasyRemote cutover.
  - Future Node/JVM/Swift adapter reports.

## Invariants

1. The SPEC remains unchanged.
2. Rust report records must be closed over every case that declares `rust`.
3. Evidence must be repository-local and use `rust_test`.
4. Rust evidence should point at daemon SDK contract modules or Rust tests, not
   language facade wrappers.
5. No Rust report record may target a case not declared for `rust`.

## Implementation Steps

1. Add `sdk/conformance/runner/rust-action-adapter-report.json`.
2. Extend runner repository-report tests to include Rust.
3. Add the Rust report to scaffold JSON/file guards.
4. Update conformance docs and SDK status language to name the P0 report set.

## Verification

- `cargo test --bin sdk-conformance-runner`
- `cargo run --bin sdk-conformance-runner -- --language rust --adapter-report sdk/conformance/runner/rust-action-adapter-report.json --format json`
- `cargo test --lib --features axon-pb _contract`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `git diff --check`

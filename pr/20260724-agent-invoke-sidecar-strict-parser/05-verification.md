# Verification

## Commands

- `cargo test parse_rejects --features axon-pb`
  - Result: passed. Covered the converted `<agent>.invoke` parser failure-path tests.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - Result: passed. Covered the new mutation fixture that reintroduces underscore sidecar parser compatibility.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - Result: passed.
- `cargo fmt --all`
  - Result: applied rustfmt to the touched Rust source.

## Evidence

The parser now rejects all unknown top-level fields, including underscore-prefixed runtime sidecars. The audit line keeps the historical `request_id` and `caller_ura` keys present as `null`, but no longer sources them from hidden ability args.

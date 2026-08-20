# Verification

## Planned checks

- `cargo fmt --check`
- `cargo test --features axon-pb failed_dispatch_result --lib`
- `cargo test --features axon-pb bidi_dispatcher --lib`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `codegraph query failed_dispatch_result --limit 40`
- `rg -n "fallback_code|let fallback =|unary map as fallback" src/daemon/invocation/bidi/bidi_dispatcher.rs`

## Results

- `cargo fmt --check` — passed.
- `cargo test --features axon-pb failed_dispatch_result --lib` — passed; 2
  targeted tests selected and passed.
- `cargo test --features axon-pb bidi_dispatcher --lib` — passed; 18 tests
  selected and passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `bash tools/scripts/check-architecture-convergence.sh` — passed.
- `git diff --check` — passed.
- `codegraph index .` — passed; indexed 1,018 files.
- `codegraph query failed_dispatch_result --limit 40` — found the helper with
  `default_code: &str` and the two targeted tests.
- `codegraph query fallback_code --limit 40` — no results.
- `rg -n "fallback_code|let fallback =|unary map as fallback"
  src/daemon/invocation/bidi/bidi_dispatcher.rs` — no matches.

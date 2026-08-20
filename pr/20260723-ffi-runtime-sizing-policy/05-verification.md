# Verification

## Planned checks

- `cargo fmt --check`
- `cargo test --features axon-pb ffi_worker_threads --lib`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `codegraph query MIN_FFI_WORKER_THREADS --limit 40`
- `codegraph query FALLBACK_FFI_WORKER_THREADS --limit 40`

## Results

- `cargo fmt --check` passed.
- `cargo test --features axon-pb host_default_ffi_worker_threads_respects_minimum --lib` passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.
- `bash tools/scripts/check-architecture-convergence.sh` passed.
- `git diff --check` passed.
- `codegraph query MIN_FFI_WORKER_THREADS --limit 40` found the new constant.
- `codegraph query FALLBACK_FFI_WORKER_THREADS --limit 40` returned no results.

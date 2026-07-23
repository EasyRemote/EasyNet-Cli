# API Contract

## Public API

- No C ABI symbols change.
- `EASYNET_FFI_WORKER_THREADS` remains the external override.

## Internal Rust API

- `MIN_FFI_WORKER_THREADS` is the lower bound for automatic worker sizing.
- `host_default_ffi_worker_threads()` computes automatic host-side sizing.

## Error behavior

Runtime construction failures still report `FFI: tokio runtime build failed`.

# Architecture

## Layering

- `ffi::client::handle` owns process-local C ABI session handles and the
  library-internal Tokio runtime.
- Runtime sizing is host-side FFI policy, not device runtime behavior.
- ABI callers remain synchronous consumers of the FFI layer; they do not own
  runtime lifecycle decisions.

## Boundary proof

`FALLBACK_FFI_WORKER_THREADS` made the minimum worker count look like a legacy
fallback. `device_default_ffi_worker_threads` made a generic FFI runtime policy
look device-owned. Both names are boundary leaks. Renaming them preserves
behavior while aligning ownership with the FFI host runtime.

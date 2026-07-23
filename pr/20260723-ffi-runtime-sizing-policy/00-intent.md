# Intent

## Goal

Remove fallback/product vocabulary from FFI client runtime sizing. The FFI
runtime owns a host-side worker policy for synchronous C ABI callers; it should
not describe the minimum worker count as a fallback or as a device default.

## Non-goals

- Do not change C ABI behavior.
- Do not rename the public `EASYNET_FFI_WORKER_THREADS` environment variable.
- Do not change runtime construction, handle lifecycle, or IPC semantics.

## Acceptance criteria

- FFI worker-count constants and helper names describe host/runtime policy.
- The runtime header no longer describes synchronous ABI calls as legacy.
- The canonical convergence gate rejects reintroduction of the retired names.
- Formatting, targeted tests/checks, and convergence gates pass.

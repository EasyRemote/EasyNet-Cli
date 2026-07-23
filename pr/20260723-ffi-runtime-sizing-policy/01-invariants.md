# Invariants

## Semantic invariants

- The FFI runtime remains process-local and multi-threaded.
- Reader tasks still make progress while synchronous ABI calls block on IPC.
- `EASYNET_FFI_WORKER_THREADS` still overrides automatic sizing when positive.

## Safety invariants

- The automatic worker count keeps the existing minimum of four workers.
- Invalid or missing environment values still use host-derived sizing.
- No compatibility alias preserves the retired fallback/device helper names.

## Boundedness invariants

- The runtime is still constructed once through `OnceLock`.
- The worker count calculation remains deterministic for a given host parallelism
  and environment value.

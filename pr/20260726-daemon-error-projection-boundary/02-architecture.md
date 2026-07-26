# Architecture

Layer ownership:
- `daemon::boot::error` owns the daemon SDK failure surface.
- `DaemonInvocationErrorProjection` owns classification of daemon invocation failures for external adapters.
- `ffi::invocation` maps the projection to stable ABI integer codes and JSON error records.

Boundary rule:
- FFI may match enum variants and transport status codes.
- FFI must not parse daemon message text to infer runtime semantics.

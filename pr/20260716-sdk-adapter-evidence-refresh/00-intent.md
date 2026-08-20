# Intent

## Slice

Refresh SDK adapter conformance evidence after the FFI invocation cleanup
refactor.

## Root Fork

The committed FFI cleanup refactor changed `src/ffi/invocation/mod.rs`, but
the C-ABI and Rust SDK adapter reports still pinned the previous source digest.
That leaves cutover readiness split between current source and stale proof
metadata.

## Expected Effect

- Architecture convergence: conformance evidence is tied to the current
  source-of-truth implementation.
- Architecture cleanliness: no manual digest editing; evidence is refreshed by
  the repository-owned tool.
- Product acceleration: SDK cutover gates can evaluate behavior instead of
  failing on stale metadata.

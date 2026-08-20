# API Contract

## Public Behavior

No exported FFI symbols change. `easynet_shutdown` continues to cancel
outstanding invocation resources for the closing session.

## Internal Boundary

Production cleanup accepts `ClientSessionBinding`, not `EasynetHandle`.
Tests may allocate handles to construct bindings, but cleanup assertions call
the binding-owned function.

## Compatibility

No compatibility shim or fallback path is retained.

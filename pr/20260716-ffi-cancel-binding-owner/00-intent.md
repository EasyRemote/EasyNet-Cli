# Intent

## Slice

Remove the obsolete handle-based FFI invocation cleanup helper and keep
shutdown cleanup owned by `ClientSessionBinding`.

## Root Fork

FFI shutdown now resolves a generation-aware `ClientSessionBinding` before
canceling invocation, stream, and bidi registries. The remaining
`cancel_invocations_for_handle` helper reintroduced a handle-only lookup path
that production no longer uses and that weakens the ownership signal.

## Expected Effect

- Architecture convergence: cleanup ownership stays on `ClientSessionBinding`,
  not raw numeric handles.
- Architecture cleanliness: remove the unused production helper instead of
  suppressing the warning.
- Product acceleration: production compile output loses the final known
  warning in this area.

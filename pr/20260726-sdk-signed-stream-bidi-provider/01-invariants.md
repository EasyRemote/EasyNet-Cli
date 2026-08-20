# Invariants

1. Signed stream/bidi submission must preserve the signed envelope; it must not rebuild or re-sign an unsigned draft.
2. RuntimeProvider stream/bidi methods are provider-backed when the wrapped RuntimeClient is present.
3. Missing RuntimeClient remains a typed provider readiness error.
4. Transport ownership stays in RuntimeClient; AuthorizedRuntimeSession does not reach into transport internals.
5. Go and Python SDKs expose the same runtime capability state.

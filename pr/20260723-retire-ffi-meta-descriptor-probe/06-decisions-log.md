# Decisions Log

## 2026-07-23

- Decision: remove the `meta.list_abilities` probe instead of making its errors
  nicer.
- Reason: descriptor resolution must not perform hidden invocation. Better error
  projection would preserve the wrong authority path and keep product failures
  timing-dependent.
- Decision: delete the generic provider-row parser with the meta probe.
- Reason: there is no explicit provider-backed descriptor catalog seam in this
  implementation slice. Keeping the parser would preserve an ownerless extension
  point for the same hidden provider architecture.
- Decision: keep `meta.list_abilities` as a normal ability only.
- Reason: products may still call it explicitly through the runtime model, but
  descriptor resolution must not call it implicitly.

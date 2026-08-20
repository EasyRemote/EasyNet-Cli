# Decisions

- 2026-07-07: Implement Node Events as a seam over injected transport methods,
  not as a daemon provider.
- 2026-07-07: Reuse `StreamHandle` for live Events profile subscriptions to
  preserve the existing bounded stream lifecycle.
- 2026-07-07: Declare Node for Events cases only after adding direct test
  evidence in `sdk/node/test/runtime-core.test.mjs`.

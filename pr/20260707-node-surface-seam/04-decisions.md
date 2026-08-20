# Decisions

- 2026-07-07: Implement Node Surface as a seam over injected transport methods,
  not as a daemon provider or page renderer.
- 2026-07-07: Keep `SurfaceStatus` semantically identical to `SurfaceHealth`
  to match the shared conformance expectation.
- 2026-07-07: Declare Node for the shared Surface case only after adding direct
  evidence in `sdk/node/test/runtime-core.test.mjs`.

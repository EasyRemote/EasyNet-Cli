# Decisions

- 2026-07-07: Implement Node Admin + Gateway as a seam over injected transport
  methods, not as a backend or daemon lifecycle provider.
- 2026-07-07: Preserve daemon readiness fields exactly in GatewayStatus.
- 2026-07-07: Declare Node for the shared Admin + Gateway case only after
  adding direct evidence in `sdk/node/test/runtime-core.test.mjs`.

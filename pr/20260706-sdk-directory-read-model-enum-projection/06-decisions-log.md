# Decisions Log

## 2026-07-06

- Keep enum projection in the Go SDK Directory/Identity profile because backend node-listing consumers should not import raw Axon SDK enum helpers.
- Preserve unknown ordinal rendering as decimal strings instead of failing closed; read-model pages are diagnostic/user-facing projections, not admission-critical validation boundaries.
- Add a small projector type rather than a backend-shaped map helper. The SDK owns enum value normalization; backend owns its local schemaless map extraction until the daemon transport adapter is fully cut over.

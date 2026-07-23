# Decisions Log

## 2026-07-24

- Keep `invocation_endpoint` validation at runtime connector resolution so
  control-only discovery still maps to the existing `CONTROL_ONLY` error.
- Reject old or partially written discovery files earlier at SDK decode.
- Treat `daemon_identity` as a strict decoded daemon discovery field, but do
  not project it as a product-specific SDK abstraction.
- Keep `pages_port` optional because Pages may not be enabled, but reject zero
  and out-of-range TCP ports when the daemon publishes the field.

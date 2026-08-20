# Invariants

- Package metadata is evidence of package shape, not evidence of stable release
  support.
- P1 language packages remain seam-labeled until provider transports and
  cutover evidence exist.
- Go and Python remain P0 provider-backed facades, but product cutover still
  requires external repository deletion and route/live-smoke evidence.
- Package manifests must not introduce product-specific SDK naming.
- The aggregate readiness gate must fail if a shipped SDK package manifest
  drifts from the canonical package identity.

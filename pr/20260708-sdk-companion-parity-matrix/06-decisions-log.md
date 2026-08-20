# Decisions Log

- 2026-07-08: Use `runtime_companion_control` rather than a product name. The
  SDK facade wraps daemon runtime control DTOs and does not own OS supervisor
  or product policy.
- 2026-07-08: Keep the shared case scoped to Go/Python because the existing
  parity validator is explicitly the Go/Python matrix gate. Swift and Java have
  their own seam tests, but this matrix does not claim four-language parity.

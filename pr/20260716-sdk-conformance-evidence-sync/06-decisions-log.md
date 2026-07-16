# Decisions Log

2026-07-16:

- Chose evidence synchronization instead of validator relaxation. The concrete
  use case is preserving live parity as a proof of the current source tree.
- Kept runtime behavior untouched; the failure was proof metadata drift, not an
  SDK lifecycle or transport behavior defect.
- Fresh live report generation is required after runner evidence changes because
  live records carry their own evidence hashes. Reusing stale `target/` results
  is intentionally rejected.

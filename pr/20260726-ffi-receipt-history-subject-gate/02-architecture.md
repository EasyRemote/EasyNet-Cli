Layering
========

- SDK receipt providers already own receipt-history authority validation.
- The FFI descriptor resolver owns request-shape validation before returning a
  descriptor reference to product callers.
- Daemon/Axon admission remains the final authority for signed invocation
  tuples.

Boundary correction
===================

The previous FFI resolver validated provider family but ignored `subject_ura`.
That allowed products to resolve `invocation.history.list` for a Device subject
and fail later with `AUTHORITY_SUBJECT_MISMATCH`. The corrected boundary rejects
non-runtime-state-read subjects before descriptor projection.

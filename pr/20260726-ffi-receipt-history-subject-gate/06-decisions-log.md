Decisions
=========

- 2026-07-26: Treat FFI receipt-history descriptor resolution as a request-shape
  gate. It must reject subject tuples that SDK receipt providers and runtime
  admission cannot authorize.
- 2026-07-26: Keep `provider: "ability_descriptor"` unchanged for catalogue
  reads. Only `provider: "receipt_history"` receives the runtime-state subject
  gate because receipt-history authority is session/delegation subject-bound.
- 2026-07-26: Extend the SPEC v2 gate for this invariant because the previous
  gate allowed FFI descriptor resolution to return receipt-history descriptors
  for Device subjects.

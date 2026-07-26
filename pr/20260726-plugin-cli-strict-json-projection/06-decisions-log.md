# Decisions Log

- Decision: use strict helper functions in the CLI rather than reintroducing typed daemon DTO deserialization.
  - Reason: the CLI owns rendering, not daemon report state.
- Decision: keep optional companion fields lossy for absent companion projections.
  - Reason: packages without companion metadata legitimately render `-`; malformed required fields still fail.


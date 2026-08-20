Execution checklist
===================

- [x] Change realm-scope catalogue source default to `authority:broadcast`.
- [x] Update `meta.list_abilities` schema text and tests.
- [x] Update nearby federation comments that describe catalogue publication as
  hub-published.
- [x] Update SPEC v2 gate to require `authority:broadcast` and reject
  `hub:broadcast`.
- [x] Run targeted meta/catalog tests.
- [x] Run fmt, diff check, and convergence gates.

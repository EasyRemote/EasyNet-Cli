Goal
====

Retire the enum-level default for descriptor receipt semantics.

Non-goals
=========

- Do not change operational receipt behavior for existing descriptors.
- Do not change state-transition receipt semantics.
- Do not change descriptor JSON/TOML shapes.

Acceptance criteria
===================

- `ReceiptSemantics` no longer implements `Default`.
- `ReceiptSemantics::Operational` remains an explicit constructor-selected
  state.
- SPEC v2 rejects reintroducing an implicit operational receipt default.

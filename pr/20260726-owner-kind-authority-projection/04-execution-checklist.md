Execution checklist
===================

- [x] Change `OwnerKind::RealmAuthority.authority_projection()` to
  `authority`.
- [x] Change `owner_kind_from_projection` to accept `authority`.
- [x] Add rejection coverage for retired `hub` owner projection.
- [x] Update descriptor/control-plane comments that still describe runtime
  owner grammar as `hub`.
- [x] Update SPEC v2 gate to enforce the `authority` parser and reject `hub`.
- [x] Run targeted dispatch tests.
- [x] Run fmt, diff check, and convergence gates.

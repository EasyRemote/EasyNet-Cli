Execution checklist
===================

- [x] Change `OwnerProjection::parse` to accept `authority` for
  `RealmAuthority`.
- [x] Change `OwnerProjection::canonical` to render `authority`.
- [x] Update authority scope tests to reject retired `hub`.
- [x] Update SPEC v2 gate to enforce `authority`, not `hub`.
- [x] Run targeted authority tests.
- [x] Run fmt, diff check, and convergence gates.

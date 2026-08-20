Execution checklist
===================

- [x] Change federation join envelope construction to canonical membership
      caller.
- [x] Replace pseudo-caller digest proof with membership caller proof.
- [x] Key bootstrap candidate leases by canonical caller URA.
- [x] Remove or migrate tests that require non-URA caller identity.
- [x] Run targeted federation join tests.
- [x] Run fmt, diff check, and architecture gates.

# Execution checklist

- [x] Inspect reset/config/keyring state ownership.
- [x] Add explicit purge flag and local-state root deletion abstraction.
- [x] Add tests for purge mode and guard behavior.
- [x] Add SPEC/source gate preventing reset from returning to credentials-only
      cleanup under purge.
- [x] Run fmt, targeted tests, and convergence gates.

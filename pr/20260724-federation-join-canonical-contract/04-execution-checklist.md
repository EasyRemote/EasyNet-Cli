# Execution Checklist

- [x] Remove `pairing_secret` from daemon-published `federation.join` schema.
- [x] Remove `pairing_secret` from client `JoinArgs`.
- [x] Migrate all constructors.
- [x] Replace tests that prove optional emission with tests proving retired field absence/rejection.
- [x] Run targeted Rust tests.
- [x] Run formatting and convergence checks proportional to this cut.

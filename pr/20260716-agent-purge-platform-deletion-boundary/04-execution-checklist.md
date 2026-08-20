# Agent Purge Platform Deletion Execution Checklist

- [x] Inspect capability-state and publication boundaries before code edits.
- [x] Inspect current purge platform support and deletion helpers.
- [x] Introduce `PlatformTreeDeletion` as the only platform purge deletion owner.
- [x] Migrate handler and finalizer callers.
- [x] Remove obsolete standalone support/deletion helpers.
- [x] Add architecture convergence gate coverage.
- [x] Run targeted Rust tests and script gates.
- [x] Commit the stable slice with `Silan.Hu <silan.hu@u.nus.edu>`.

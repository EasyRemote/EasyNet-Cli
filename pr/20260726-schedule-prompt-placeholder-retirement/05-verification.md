Verification
============

Planned checks
--------------

- Targeted Rust tests for schedule service/store/ability handlers.
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

Results
-------

- `cargo test schedule --lib --bins`: passed; 38 lib schedule-filtered tests
  and 1 daemon-bin schedule-filtered test.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`: passed.

Gate update
-----------

The existing SPEC v2 schedule-store gate still required serialization to insert
`prompt: null`. That requirement was itself a compatibility seam, so the gate
was upgraded to reject prompt-null serialization and require missing/null/blank
prompt negative vectors.

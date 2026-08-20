# Verification

All planned checks passed on 2026-07-23.

- `cargo fmt --check`
  - Result: passed.
- `cargo test -q descriptor_resolution_errors_project_canonical_runtime_codes --lib`
  - Result: passed, 1 test.
- `cargo test -q runtime_descriptor_remote_probe_requires_caller_signer_before_daemon_io --lib`
  - Result: passed, 1 test.
- `cargo test -q runtime_descriptor_remote_probe_requires_runtime_owner_identity --lib`
  - Result: passed, 1 test.
- `bash tools/scripts/check-architecture-convergence.sh`
  - Result: `architecture-convergence: OK`.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - Result: `canonical-runtime-convergence-v2: OK`.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
  - Result: synced changed FFI and remote invocation sources.
- `/Users/macbook.silan.tech/.local/bin/codegraph query from_remote_probe_failure`
  - Result: no results found.
- `/Users/macbook.silan.tech/.local/bin/codegraph callers invoke_remote_target_with_caller_signer_typed`
  - Result: one caller, `RemoteDescriptorCatalogProbe::invoke`.

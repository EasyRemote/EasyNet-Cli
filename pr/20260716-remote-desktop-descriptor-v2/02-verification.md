# Verification

Planned checks:

- `cargo test -q plugin_host_descriptor_projector_preserves_manifest_call_modes --lib`
- `cargo test -q remote_desktop --lib`
- `cargo test -q real_remote_desktop --lib`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`

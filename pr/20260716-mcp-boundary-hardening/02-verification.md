# Verification

- `cargo test -q bridge_tools_surface_excludes_non_rpc_descriptors --lib`
- `cargo test -q bounded_reader_rejects_oversized_eof_frame_without_retaining_extra_bytes --lib`
- `cargo test -q mcp --lib`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `git diff --cached --check`

# Verification

1. `cargo test --features axon-pb provider_excludes_geometries_it_cannot_invoke --lib`
2. `cargo test --features axon-pb tool_specs_lists_every_rpc_descriptor_passed_at_construction --lib`
3. `bash tests/scripts/test_check_architecture_convergence.sh`
4. `tools/scripts/check-architecture-convergence.sh`
5. `git diff --check`

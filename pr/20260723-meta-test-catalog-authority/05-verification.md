# Verification

Passed:

- `cargo fmt --check`
- `git diff --check`
- `cargo test -q combined_runtime_projects_disjoint_device_and_hub_views_from_callee --features axon-pb`
- `cargo test -q list_abilities_surfaces_dynamic_manifest_schema_for_hot_registered_tools --features axon-pb`
- `cargo test -q governance::meta::tests --features axon-pb`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
- `/Users/macbook.silan.tech/.local/bin/codegraph status`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

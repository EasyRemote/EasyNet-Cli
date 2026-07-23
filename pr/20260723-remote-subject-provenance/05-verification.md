# Verification

Completed.

Commands:

- `bash tools/scripts/check-daemon-invocation-migration.sh`
- `python3 sdk/conformance/edge_adapter_policy.py --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `cargo test public_tuple_plan_preserves_explicit_tuple_facts --features axon-pb`
- `cargo test remote_system_issuer_names_system_root_derivation --features axon-pb`
- `cargo test runtime_descriptor_remote_probe --features axon-pb`
- `/Users/macbook.silan.tech/.local/bin/codegraph index .`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "RemoteInvocationSubject" --limit 30`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "TargetOwnedSystem" --limit 30`

Results:

- Daemon invocation migration gate passes.
- Canonical runtime convergence v2 self-test passes.
- Canonical runtime convergence v2 main gate passes.
- Legacy architecture convergence gate passes.
- Targeted Rust tests pass.
- Codegraph indexes the workspace and finds the remote subject provenance state machine.
- Codegraph reports no `TargetOwnedSystem` symbol results.

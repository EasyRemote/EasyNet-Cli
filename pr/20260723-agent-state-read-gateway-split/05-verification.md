# Verification

Passed:

- `bash tools/scripts/check-runtime-state-read-subject-boundary.sh`
- `bash tests/scripts/test_check_runtime_state_read_subject_boundary.sh`
- `cargo fmt --check`
- `git diff --check`
- `cargo check -q --features axon-pb`
- `cargo test -q run_set_missing_root_path_fails_before_agent_start_is_sent --features axon-pb`
- `cargo test -q run_publish_dry_run_succeeds_on_a_fresh_agent --features axon-pb`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
- `/Users/macbook.silan.tech/.local/bin/codegraph status`

Source scan:

- No production `gateway.invoke("agent.list" | "meta.list_abilities")`
  matches remain under the Agent CLI.
- No production `invoke_local_ability("agent.list" | "meta.list_abilities" |
  "node.describe")` matches remain under CLI/runtime-state read surfaces.

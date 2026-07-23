# Verification

Passed:

- `bash tools/scripts/check-runtime-state-read-subject-boundary.sh`
- `bash tests/scripts/test_check_runtime_state_read_subject_boundary.sh`
- `cargo fmt --check`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
- `/Users/macbook.silan.tech/.local/bin/codegraph status`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo test -q runtime_state_read_subject --features axon-pb`

Source scan:

- No product call sites remain for
  `invoke_local_ability("meta.list_resources" | "meta.list_abilities" |
  "observe.health" | "agent.list" | "invocation.history.*" |
  "identity.list_user_pubkeys")`; the only remaining match is the
  `local_invoke.rs` daemon-down unit test.

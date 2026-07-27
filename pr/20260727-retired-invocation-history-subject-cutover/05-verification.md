Planned verification:
- `cargo test -q --features axon-pb retired_invocation_history_subject`
- `cargo test -q --features axon-pb session_authority`
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

Results: pending.
Results:
- PASS: `cargo test -q --features axon-pb retired_invocation_history_subject`
- PASS: `cargo test -q --features axon-pb session_authority_rejects_request_scoped_retired_invocation_history_subject_carrier`
- PASS: `cargo test -q --features axon-pb authority_subject_kind_accepts_only_canonical_user_or_session_resources`
- PASS: `cargo fmt --check`
- PASS: `git diff --check`
- PASS: `bash tools/scripts/check-architecture-convergence.sh`
- PASS: `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

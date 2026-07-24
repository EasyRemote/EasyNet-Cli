# Verification

## Static analysis

- `codegraph status /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`
- `codegraph sync /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`
- `codegraph explore all-zero principal`

Result: codegraph 1.4.1 indexed and synced the current worktree. The user-provided override path was absent in this environment, so the existing installed binary at `/Users/macbook.silan.tech/.local/bin/codegraph` was used.

## Focused Rust tests

- `cargo test exact_all_zero_principal_id_trims_and_ignores_case --features axon-pb -- --nocapture`
- `cargo test embedded_all_zero_principal_placeholder_detects_ura_fields --features axon-pb -- --nocapture`
- `cargo test parse_invocation_json_rejects_all_zero_subject_before_daemon_io --features axon-pb -- --nocapture`
- `cargo test auth_session_rejects_all_zero_user_id_owner_fact --features axon-pb -- --nocapture`
- `cargo test load_credentials_rejects_all_zero_user_id --features axon-pb -- --nocapture`
- `cargo test delegation_payload_rejects_all_zero_principal_placeholders --features axon-pb -- --nocapture`

Result: all focused tests passed.

## Architecture gates

- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

Result: all gates passed.

## Formatting and diff hygiene

- `cargo fmt --all`
- `cargo fmt --check`
- `git diff --check`

Result: all checks passed.

# Verification

Verified on 2026-07-23:

- `cargo fmt --check`
- `cargo test -q parse_invocation_json_rejects_all_zero_subject_before_daemon_io --lib`
- `cargo test -q parse_invocation_json_rejects_session_authority_subject_mismatch_before_daemon_io --lib`
- `cargo test -q parse_invocation_json_supports_complete_bidi_invocation --lib`
- `cargo test -q invocation_builder_emits_complete_bidi_frame0 --lib`
- `cargo test -q session_authority_binds_subject_resource_to_declared_owner_and_session --lib`
- `cargo test -q post_admission_projection_preserves_the_verified_payload --lib`
- `cargo test -q parse_invocation_json --lib`
- `cargo test -q authority_metadata --lib`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `codegraph sync .`
- `codegraph callers project_invocation_authority_metadata_shape`
- `codegraph callers session_authority_admits_subject`
- `rg '"x-easynet-delegation".*producer' src sdk` returned no matches.

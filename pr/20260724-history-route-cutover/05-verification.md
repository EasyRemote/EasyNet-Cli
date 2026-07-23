# Verification

## Planned

- Rust unit test for target-owned remote system facade rejecting `invocation.history.list`.
- Rust unit test for target-owned remote system facade continuing to project `meta.list_abilities`.
- SPEC v2 gate check for absence of history routing through `invoke_remote_device_system_ability`.
- `cargo fmt --check`.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`.
- `bash tools/scripts/check-architecture-convergence.sh`.
- `git diff --check`.

## Results

- `cargo test remote_system_issuer --features axon-pb` — passed.
- `cargo fmt --check` — passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `bash tools/scripts/check-daemon-invocation-migration.sh` — passed.
- `bash tools/scripts/check-architecture-convergence.sh` — passed.
- `git diff --check` — passed.
- Production search for `RemoteSystemInvocationIssuer::root_plan`, `pub(crate) fn root_plan<'a>`, and `target_owned_system_subject_ura` found no production source hits; remaining hits are gate legacy/self-test fixtures.

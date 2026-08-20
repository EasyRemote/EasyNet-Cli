Verification matrix
===================

Planned checks:

- `cargo test invoke_dispatches_federation_join_to_wrapper --lib`
- `cargo test federation_join_with_principal_proof_binds_device_owner_in_runtime_trust --lib`
- `cargo test federation_join_bootstrap_uses_membership_ura_as_caller --lib`
- `cargo test daemon::invocation::dispatch::daemon_route_runtime::bootstrap_join_proof_tests --lib`
- `cargo test bootstrap_candidate_key_lease_requires_device_caller_and_removes_on_drop --lib`
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

Current evidence:

- `codegraph sync .` refreshed the analysis index after deleting the legacy
  caller helper module.
- `rg -n "provisional_ura_for_pubkey|federation_join_genesis|ProvisionalJoinProof|provisional:|provisional caller|provisional identity|provisional bootstrap|provisional_ura|ProvisionalBootstrap|provisional_bootstrap|_provisional_key_lease|provisional_key_lease" src tests tools pr/20260726-canonical-federation-join-caller --glob '!target/**'`
  returned no matches.
- `cargo test federation_join_bootstrap_uses_membership_ura_as_caller --lib`
  passed.
- `cargo test daemon::invocation::dispatch::daemon_route_runtime::bootstrap_join_proof_tests --lib`
  passed.
- `cargo test invoke_dispatches_federation_join_to_wrapper --lib` passed.
- `cargo test federation_join_with_principal_proof_binds_device_owner_in_runtime_trust --lib`
  passed.
- `cargo test bootstrap_candidate_key_lease_requires_device_caller_and_removes_on_drop --lib`
  passed.
- `cargo test daemon::invocation::dispatch::daemon_invocation_service::tests::unary::invoke_returns_invalid_argument_on_bad_json --lib`
  passed after restricting bootstrap ingress to canonical hub join tuples.
- `cargo test daemon::invocation::dispatch::daemon_invocation_service::tests::unary --lib`
  passed: 65 passed, 0 failed.
- `cargo fmt --check` passed.
- `git diff --check` passed.
- `bash tools/scripts/check-architecture-convergence.sh` passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.

No remaining verification gap for this iteration.

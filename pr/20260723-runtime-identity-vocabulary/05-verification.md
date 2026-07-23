# Verification

## Checks

- `cargo test daemon::ability::authority::tests`
- `cargo test daemon::identity::local_invocation::tests`
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `codegraph index .`
- `codegraph query "Product URA" --limit 20`
- `codegraph query "Product-level authority fact" --limit 20`
- `rg -n "Product URA|Product-level authority fact|product-level authority fact" src/daemon/identity/local_invocation.rs src/daemon/ability/authority/mod.rs || true`

## Results

- `cargo test daemon::ability::authority::tests`: passed, 13 tests.
- `cargo test daemon::identity::local_invocation::tests`: passed, 7 tests.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `codegraph index .`: passed, indexed 1,018 files.
- `codegraph query "Product URA" --limit 20`: returned symbol-level fuzzy matches; no literal match in the targeted daemon runtime identity files.
- `codegraph query "Product-level authority fact" --limit 20`: returned authority symbols; no literal match in the targeted daemon runtime identity files.
- `rg -n "Product URA|Product-level authority fact|product-level authority fact" src/daemon/identity/local_invocation.rs src/daemon/ability/authority/mod.rs || true`: no matches.

## Non-blocking diagnostic

An initial broad `cargo test authority` command selected the environment-dependent
real-invoke test
`daemon::ability::builtins::real_invoke_tests::real_authority_binding_grant_list_check_and_revoke_round_trip`.
That test failed because no local hosted device projection/credentials were provisioned
in this shell. The narrower authority module test suite passed and is the relevant
verification for this vocabulary/gate slice.

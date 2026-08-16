# Ability conformance owner-binding verification

## Checks

- `cargo test combined_registry_satisfies_each_authority_baseline_without_name_only_fallback`
  - passed: 1 targeted test, 0 failures
- `cargo test daemon::ability::conformance::tests::`
  - passed: 16 tests, 0 failures
- `git diff --check`
  - passed
- `codegraph sync . && codegraph status .`
  - passed, index up to date after syncing 1 changed Rust file
- URA-only touched-file scan
  - command:
    `rg -n "\bURI\b|\buri\b" src/daemon/ability/conformance.rs pr/20260816-ability-conformance-owner-binding || true`
  - no matches

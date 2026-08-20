## Planned checks

- `cargo test --lib eal::interpreter`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`

## Results

- `cargo test --lib eal::interpreter` — passed, 49 tests.
- `cargo test eal::interpreter` — reached unrelated integration-test compile failures around `PresenceRegistry::insert`, so it is not authoritative for this internal module change.
- `cargo fmt --check` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `tools/scripts/check-architecture-convergence.sh` — passed.
- `rg 'fall back to sequential|falls back to sequential|fallback_to_sequential|clone_for_thread\(\)\.is_ok\(\)|Signal "fall back"' src/eal/interpreter` — no matches.

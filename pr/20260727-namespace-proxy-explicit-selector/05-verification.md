Verification completed:

- `cargo test namespace_proxy_resolve --lib` — pass, 10 tests.
- `cargo test namespace_resolve --lib` — pass, 8 tests.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — pass.
- `bash tools/scripts/check-daemon-invocation-migration.sh` — pass.
- `bash tools/scripts/check-transport-locator-terminology-boundary.sh` — pass.
- `bash tools/scripts/check-system-ability-retired-aliases.sh` — pass.
- `bash tools/scripts/check-architecture-convergence.sh` — pass.
- `cargo fmt --check` — pass.
- `git diff --check` — pass.
- `codegraph sync .` — pass.

Notes:

- `namespace.proxy_resolve` now rejects omitted `ability_name` instead of treating the selector as absent-by-default.
- `ability_name: null` remains the explicit directory/listing selector state.
- Empty selector strings are rejected at the selector value-object boundary.

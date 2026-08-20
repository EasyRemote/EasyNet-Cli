Verification
============

Planned checks
--------------

- `cargo test daemon::ability::descriptors::surface --lib`
- `cargo test daemon::ability::catalog::ability_toml --lib`
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

Results
-------

- `cargo test daemon::ability::descriptors::surface --lib`: passed, 43 tests.
- `cargo test daemon::ability::catalog::ability_toml --lib`: passed, 12 tests.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`: passed.

Residual search
---------------

`rg` found no production `Visibility::default()`, `ScopeRule::default()`,
`#[default] Private`, or `#[default] None` paths. The remaining matches are the
SPEC v2 rejection strings.

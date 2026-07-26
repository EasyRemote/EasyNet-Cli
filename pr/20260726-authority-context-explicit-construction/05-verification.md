Verification
============

Planned checks
--------------

- `cargo test ability_authority_context --lib`
- `cargo test daemon::ability::catalog::assembly_tests --lib`
- `cargo test selected_route_descriptor_ref_comes_from_live_catalog_for_all_modes --lib`
- `cargo test plugin_runtime_host_hot_reload --lib`
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

Results
-------

- `cargo test ability_authority_context --lib`: passed, 0 matching tests after
  compiling the library; useful as a compile check for the removed `Default`
  implementation.
- `cargo test daemon::ability::catalog::assembly_tests --lib`: passed, 42
  tests.
- `cargo test selected_route_descriptor_ref_comes_from_live_catalog_for_all_modes --lib`:
  passed, 1 test.
- `cargo test plugin_runtime_host_hot_reload --lib`: passed, 3 tests.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`: passed.

Failure-path evidence
---------------------

The first compile attempt failed because `AxonAbilityCatalog` derived
`Default`, proving the ambient authority default was still structurally
reachable. The fix removed the catalog `Default` derive and the ambient
metadata-only `new()` constructor instead of restoring compatibility.

Assembly tests then exposed stale test fixtures that still wrote shorthand
AgentRegistry keys and device-scoped authority context. Those fixtures now use
canonical registry keys, explicit root paths, and a hosted-agent authority
inventory derived from the fixture registry.

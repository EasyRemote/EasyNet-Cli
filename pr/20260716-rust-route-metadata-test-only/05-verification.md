# Verification Plan

```bash
python3 provider_routes/generate_principal_routes.py --check
python3 provider_routes/generate_access_control_routes.py --check
python3 provider_routes/generate_receipt_routes.py --check
python3 provider_routes/generate_runtime_admin_routes.py --check
cargo test routes_are_generated_from_manifest --lib
cargo test --features axon-pb --lib --no-run 2>&1 | tee target/rust-route-metadata-no-run.log
rg 'constant `.*_(PROFILE|ROUTE_MANIFEST_SHA256)` is never used' target/rust-route-metadata-no-run.log
bash tools/scripts/check-architecture-convergence.sh
git diff --check -- provider_routes/route_generator.py src/cli/commands/groups/principal_routes_gen.rs src/daemon/ability/access_control_routes_gen.rs src/daemon/ability/principal_routes_gen.rs src/daemon/ability/receipt_routes_gen.rs src/daemon/ability/runtime_admin_routes_gen.rs pr/20260716-rust-route-metadata-test-only
```

The warning grep is expected to return no matches.

## Results

- `python3 provider_routes/generate_principal_routes.py --check`
  `&& python3 provider_routes/generate_access_control_routes.py --check`
  `&& python3 provider_routes/generate_receipt_routes.py --check`
  `&& python3 provider_routes/generate_runtime_admin_routes.py --check`: pass.
- `cargo test routes_are_generated_from_manifest --lib`: pass; 4 route
  manifest freshness tests passed.
- `cargo test --features axon-pb --lib --no-run 2>&1 | tee target/rust-route-metadata-no-run.log`: pass.
- Warning grep against `target/rust-route-metadata-no-run.log`: no matches for
  unused `*_PROFILE` or `*_ROUTE_MANIFEST_SHA256` constants.
- `bash tools/scripts/check-architecture-convergence.sh`: pass.
- `git diff --check -- provider_routes/route_generator.py src/cli/commands/groups/principal_routes_gen.rs src/daemon/ability/access_control_routes_gen.rs src/daemon/ability/principal_routes_gen.rs src/daemon/ability/receipt_routes_gen.rs src/daemon/ability/runtime_admin_routes_gen.rs pr/20260716-rust-route-metadata-test-only`: pass.

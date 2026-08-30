# Verification

## Passing commands

Executed on 2026-08-23:

```bash
bash -n tools/scripts/check-remoteapp-product-closure-audit.sh tests/scripts/test_check_remoteapp_product_closure_audit.sh
bash tools/scripts/check-remoteapp-product-closure-audit.sh
cargo test --features remote-desktop,axon-pb -p easynet daemon::plugins::surface::tests::plugin_host_surface_projects_declared_bidi_wire_kind
cargo test --features remote-desktop,axon-pb -p easynet daemon::ability::builtins::real_invoke_tests::real_device_plugin_status_surfaces_remoteapp_attach_wire_kind
rustfmt --edition 2021 --check src/daemon/plugins/surface.rs
rustfmt --edition 2021 --check src/daemon/ability/builtins/real_invoke_tests.rs
git diff --check -- src/daemon/plugins/surface.rs src/daemon/ability/builtins/real_invoke_tests.rs tools/scripts/check-remoteapp-product-closure-audit.sh tests/scripts/test_check_remoteapp_product_closure_audit.sh pr/20260823-remoteapp-wire-kind-surface
bash tests/scripts/test_check_remoteapp_product_closure_audit.sh
```

## Expected proof

- The surface unit test proves typed projection of
  `metadata_json_plus_binary`.
- The closure audit proves RemoteApp attach cannot regress to JSON-only manifest,
  JSON-only compiled registration, or missing plugin surface projection.
- The real invoke test proves the daemon `plugin.status` ability returns the
  projected field on the same JSON surface consumed by CLI/frontend automation.

# Verification

Passing commands:

```bash
bash -n tools/scripts/check-remoteapp-product-closure-audit.sh tests/scripts/test_check_remoteapp_product_closure_audit.sh
python3 -m json.tool docs/design/remoteapp-product-readiness-matrix.json
bash tools/scripts/check-remoteapp-product-closure-audit.sh
cargo test --features remote-desktop,axon-pb -p easynet daemon::plugins::manifest::tests::manifest_accepts_metadata_json_plus_binary_bidi_wire_kind
cargo test --features remote-desktop,axon-pb -p easynet daemon::ability::wire::tests::plugin_metadata_json_plus_binary_maps_to_binary_capable_json_adapter
targeted RemoteApp wire-kind mutation checks: ok
rustfmt --edition 2021 --check src/daemon/plugins/manifest.rs src/daemon/ability/wire/mod.rs
git diff --check -- src/daemon/plugins/manifest.rs src/daemon/ability/wire/mod.rs plugins/remote-desktop/plugin.toml plugins/remote-desktop/src/registration.rs tools/scripts/check-remoteapp-product-closure-audit.sh tests/scripts/test_check_remoteapp_product_closure_audit.sh docs/design/remoteapp-product-readiness-matrix.json pr/20260823-remoteapp-bidi-wire-kind
```

These checks prove the product wire-kind declaration and fail-closed audit. They do not prove live audio/video product readiness.

# Verification

Planned focused checks:

- PASS: `cargo check --features remote-desktop,axon-pb -p easynet`
- PASS: `cargo test --features remote-desktop,axon-pb -p easynet daemon::plugins::descriptor::tests::plugin_host_descriptor_projector_preserves_manifest_call_modes`
- PASS: `cargo test --features remote-desktop,axon-pb -p easynet daemon::ability::catalog::ability_toml::tests::governed_contract_`
- PASS: `cargo test --features remote-desktop,axon-pb -p easynet daemon::plugins::package::tests::`
- PASS: `rustfmt --edition 2021 --check src/daemon/ability/manifest.rs src/daemon/ability/descriptors/surface.rs src/daemon/ability/catalog/ability_toml.rs src/daemon/ability/catalog/catalog_metadata.rs src/daemon/ability/catalog/system_manifest.rs src/daemon/plugins/package.rs src/daemon/plugins/descriptor.rs`
- PASS: `git diff --check`

Do not treat these checks as full RemoteApp product completion. They prove only
the route contract seam for bidi data-plane selection.

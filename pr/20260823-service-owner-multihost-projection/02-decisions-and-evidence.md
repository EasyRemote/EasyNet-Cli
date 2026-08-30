# Decisions and evidence

## Decisions

1. Keep Service as the public callee/owner for Pages-style account abilities.
2. Do not project Service owners as Agent/SystemAgent directory rows.
3. Store Service owner projections per host Device placement so same-user multi-device publication is not rejected as an owner conflict.
4. Resolve Service routes by selecting a live same-realm host Device row before constructing the final route.

## Evidence

- `cargo test -q --features axon-pb service_owner_projection_is_fenced_per_host_device`
- `cargo test -q --features axon-pb service_owner_projection_selects_live_host_from_multihost_rows`
- `cargo test -q --features axon-pb service_owner_projection`
- `cargo test -q --features axon-pb handle_advertise_abilities`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `git diff --check`

# Companion Local-Only Verification

## Commands

```text
cargo fmt
cargo test -q companion_control_abilities_are_local_only
cargo test -q descriptors_for_excludes_local_companion_control
cargo test -q local_runtime_authority_rejects_daemon_local_companion_control_routes
cargo test -q published_ability_names_matches_live_registry
cargo test -q published_system_abilities_excludes_plugin_package_abilities
cargo test -q device_owned_ability_online_resolves_final_local_device_route
cargo test -q descriptors_for_emit_only_owned_names
cargo test -q daemon::invocation::routing::route_resolver
git diff --check
rg -n "\b[U]R[I]\b|\bu[r]i\b" src/daemon/ability/catalog src/daemon/invocation/routing/route_resolver.rs
```

## Result

- New local-only catalog, profile, and route tests passed.
- Existing publication/profile route regressions passed.
- Full route resolver module passed.
- Whitespace and touched-file terminology checks passed.

# Remoteapp Watch Inventory Availability Invariants

1. `resource.watch_remote_targets` must distinguish a real inventory delta from an inventory-source outage.
2. A discovery-unavailable watch observation must not emit previous targets as `removed_resource_uras`.
3. The stream must expose `screen_target_discovery_available=false` and a typed event so the frontend can render retry/unavailable state.
4. Real removals after a successful scan must continue to emit `target_inventory_delta` with `removed_resource_uras`.
5. The watch signature must include discovery availability state so available-empty and unavailable-empty observations are not coalesced.
6. Freshness-only timestamp changes must still be ignored for stable inventory equality.
7. Inventory logic remains daemon resource-layer code; remote desktop session code remains a consumer of selected resource subjects only.

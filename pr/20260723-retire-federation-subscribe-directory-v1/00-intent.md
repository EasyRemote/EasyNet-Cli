# Intent

## Goal

Remove the retired `federation.subscribe_directory` v1 descriptor-only stream
surface from active system ability inventory.

## Non-goals

- Do not change `federation.subscribe_directory_v2`.
- Do not add a v1-to-v2 compatibility route.
- Do not preserve v1 as an alias, descriptor, or hidden route.
- Do not change cross-realm directory streaming semantics.

## Acceptance Criteria

- `ability-descriptors/system/federation/federation.subscribe_directory.ability.toml`
  is removed.
- Active descriptors may only publish the typed
  `federation.subscribe_directory_v2` stream.
- Convergence gates reject reintroduction of the retired v1 descriptor or
  active `federation.subscribe_directory` ability name.
- Stream dispatcher and cross-realm directory tests remain green for v2.

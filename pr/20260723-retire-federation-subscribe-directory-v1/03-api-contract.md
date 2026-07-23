# API Contract

## Public Behavior

- `federation.subscribe_directory_v2` remains the supported stream ability.
- `federation.subscribe_directory` is absent from active discovery.

## Error Behavior

Attempting to use the retired v1 ability should fail as absent capability or
route-not-found. It must not silently alias to v2.

## Reintroduction Rule

Any future directory-stream version must be a new explicit versioned ability
with a matching stream dispatcher branch, descriptor, tests, and migration
evidence.

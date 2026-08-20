# Architecture

## Boundary

`federation.join` is a runtime capability, not a product token-pairing API. Its public contract belongs in daemon ability catalog metadata and the federation client ability contract.

## Module ownership

- `src/daemon/ability/catalog/daemon_invocation_contracts.rs` owns the daemon-published input schema.
- `src/daemon/federation/client/ability_contract.rs` owns the CLI/client projection used to construct canonical join arguments.
- `src/daemon/invocation/dispatch/federation_wrappers.rs` owns hub-side canonical request parsing and receipt generation.

## Refactoring direction

Remove `pairing_secret` from the runtime model instead of preserving it as an ignored optional field. If a product wants token pairing, it must remain outside the canonical SDK/runtime join contract.

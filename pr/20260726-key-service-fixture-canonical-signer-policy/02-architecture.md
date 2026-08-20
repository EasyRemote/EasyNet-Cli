# Architecture

The key-service fixture is a transport substitute, not a policy owner.

Production ownership:

- `src/daemon/identity/signer_policy.rs` owns signer policy reference
  derivation.
- `src/daemon/keyring/mod.rs` enforces runtime signing projection custody.
- `tests/key_service_fixture.rs` should only expose a process-local v2 key
  service endpoint for integration tests.

The fixture therefore delegates policy derivation to the production identity
module and keeps only transport, key storage, and signature emission behavior.

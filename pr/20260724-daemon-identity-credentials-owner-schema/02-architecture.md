# Architecture

## Boundary

`src/daemon/persistence/config.rs` owns the persisted credentials schema and validation. `src/daemon/boot/invocation/identity.rs` owns binding a validated identity to the local runtime signer.

## Refactoring direction

The previous boot identity code re-declared a narrow `StoredDeviceIdentity` reader that tolerated unknown fields and carried explicit retired-field sentinel deserializers. That created a second schema authority and preserved old-data compatibility inside boot.

The converged model has boot identity load the owning `Credentials` type and project only the runtime caller URA from validated fields.

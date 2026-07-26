# Architecture

`src/cli/commands/federation_wire.rs` owns the join-time materialization of
local federation wiring. It may write:

- `[daemon].hub_endpoint` from `Credentials.hub_endpoint`, the device-to-hub
  dial target supplied by pairing.
- `[daemon.federated_peers].<realm>` from an explicit peer-hub TLS endpoint.

The resolver is a small state machine with two accepted states:

1. `OperatorOverride`
2. `PairingTlsEndpoint`

Every other state is rejected. There is no `Guessed` state after this cutover.

The SPEC v2 gate owns the regression boundary because this is an architecture
rule, not only a unit-level behavior.

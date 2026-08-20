# API Contract

## Internal API

`HostedAgentDelegationIssuer::materialize_request_metadata` receives:

- immutable request metadata,
- the canonical protobuf envelope,
- `HostedAgentDelegationIngress`,
- the selected route ability.

## Behavior

- Missing delegation request metadata returns metadata unchanged.
- `TrustedLocalSystem` plus local-system caller mints signed delegation metadata.
- `ExternalSigned` and `BootstrapCandidate` reject unsigned hosted-agent delegation request metadata with `PERMISSION_DENIED`.
- Pre-signed hosted-agent delegation metadata paired with a request remains invalid.

## Public compatibility

No public command, wire DTO, FFI symbol, or SDK method changes.

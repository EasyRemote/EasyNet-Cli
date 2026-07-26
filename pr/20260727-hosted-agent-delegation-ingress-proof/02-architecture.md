# Architecture

## Boundary

`HostedAgentDelegationIssuer` sits between daemon dispatch routing and Axon LocalRuntime wire dispatch. It does not admit invocations. It only converts a daemon-local request for hosted-agent delegation into signed metadata after the dispatch layer proves local-system ingress.

## Refactoring target

Current callers pass `true` for trusted local system and `false` for external/bootstrap paths. That is semantically weak because a raw boolean does not name the lifecycle state it represents.

The new shape introduces a small ingress state type:

- `TrustedLocalSystem`
- `ExternalSigned`
- `BootstrapCandidate`

Only `TrustedLocalSystem` can mint signed hosted-agent delegation metadata.

## Module ownership

- Dispatchers own route classification.
- `hosted_agent_delegation` owns metadata materialization and local-system envelope checks.
- Axon descriptor-bound dispatch continues to own canonical envelope reassembly and local-system signing.

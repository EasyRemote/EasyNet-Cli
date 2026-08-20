# Python Admin + Gateway Neutrality

## Objective

Remove EasyRemote-specific public naming from the Python Admin + Gateway profile while preserving daemon agent lifecycle and gateway lifecycle behavior required by `docs/spec/daemon-sdk-requirements-v1.md`.

## Boundary Proof

- Ownership: daemon agent lifecycle projections and gateway lifecycle state belong to the SDK Admin + Gateway profile, not an EasyRemote product facade.
- Runtime delegation: agent lifecycle calls continue to use `AdminClient` and `AdminCarrierBase`; gateway lifecycle remains a reusable daemon-start/config/fingerprint helper without product TLS or onboarding policy.
- Public surface: old product-named public classes are removed rather than aliased so the SDK exposes one Admin + Gateway model.
- Product boundary: EasyRemote may retain ergonomic aliases in its own package, but SDK exports must use profile-native names.

## Implementation

- Rename `EasyRemoteAgentRecord` to `AgentLifecycleRecord`.
- Rename `EasyRemoteAgentStartProjection` to `AgentStartProjection`.
- Rename `EasyRemoteAgentStopProjection` to `AgentStopProjection`.
- Rename `EasyRemoteAdminAdapter` to `AgentLifecycleAdapter`.
- Rename `EasyRemoteGatewayState` to `GatewayLifecycleState`.
- Rename `EasyRemoteGatewayDaemonHandle` to `GatewayDaemonHandle`.
- Rename `EasyRemoteGatewayConfig` to `GatewayConfig`.
- Rename `EasyRemoteGatewayRuntime` to `GatewayRuntime`.
- Rename `EasyRemoteGatewayFacade` to `GatewayLifecycleFacade`.
- Update exports, profile bridge, tests, and SDK documentation.

## Verification

- Python Admin + Gateway tests.
- Full Python SDK tests.
- Go SDK tests.
- SDK scaffold check.
- Formatting, diff, and terminology scans for removed public product names.

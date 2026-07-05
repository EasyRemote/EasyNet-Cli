# Python Mission Neutrality

## Objective

Remove EasyRemote-specific public naming from the Python Mission profile while preserving daemon mission run/track/cancel/events behavior required by `docs/spec/daemon-sdk-requirements-v1.md`.

## Boundary Proof

- Ownership: mission run projections, event projections, child Invocation fact conformance, and generic EAL mission plan rendering belong to the SDK Mission profile.
- Product boundary: EasyRemote Pipeline DSL and product scheduling policy remain outside the SDK; SDK classes use Mission/EAL terms only.
- Runtime delegation: execution still flows through `MissionClient` and `MissionCarrierBase`; no product-owned mission transport is introduced.
- Compatibility posture: old product-named public classes are removed rather than aliased so the SDK exposes one Mission profile model.

## Implementation

- Rename `EasyRemoteMissionRunProjection` to `MissionRunProjection`.
- Rename `EasyRemoteMissionEventTailer` to `MissionEventProjectionTailer`.
- Rename `EasyRemoteMissionAdapter` to `MissionExecutionAdapter`.
- Rename `EasyRemotePipelinePlan` to `MissionPlan`.
- Rename `EasyRemotePipelineStep` to `MissionPlanStep`.
- Rename `EasyRemotePipelineStepOutput` to `MissionPlanStepOutput`.
- Rename `EasyRemotePipelineChildInvocationIntent` to `MissionChildInvocationIntent`.
- Rename `EasyRemotePipelineChildInvocationConformance` to `MissionChildInvocationConformance`.
- Rename private `_easyremote_*` EAL helpers to `_mission_*`.
- Update exports, profile bridge, tests, and SDK documentation.

## Verification

- Python Mission tests.
- Full Python SDK tests.
- Go SDK tests.
- SDK scaffold check.
- Formatting, diff, and terminology scans for removed public product names.

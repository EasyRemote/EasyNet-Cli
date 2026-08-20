# Python Profile Bridge Neutrality

## Objective

Remove EasyRemote-specific public naming from the Python Admin/Mission profile bridge while preserving SDK-owned dispatch/projection behavior required by `docs/spec/daemon-sdk-requirements-v1.md`.

## Boundary Proof

- Ownership: the bridge is SDK glue over Admin and Mission DTOs plus daemon system abilities; it is not an EasyRemote product implementation.
- State: no lifecycle or mission state machine changes are introduced; existing Admin/Mission clients remain the behavior owners.
- Transport: the dispatcher protocol stays minimal and delegates system ability execution to the integration layer.
- Compatibility posture: old product-named bridge module and public symbols are removed instead of aliased, so the SDK has one profile bridge model.

## Implementation

- Rename `easyremote_profiles.py` to `profile_bridge.py`.
- Rename `EasyRemoteProfileBridge` to `DaemonProfileBridge`.
- Rename `EasyRemoteProfileDispatcher` to `ProfileBridgeDispatcher`.
- Move bridge errors onto canonical `admin_gateway` and `mission` profile stages.
- Update tests, exports, and SDK docs.

## Verification

- Python profile bridge tests.
- Python SDK test suite.
- Go SDK tests.
- SDK scaffold gate.
- Formatting, diff, and terminology scans.

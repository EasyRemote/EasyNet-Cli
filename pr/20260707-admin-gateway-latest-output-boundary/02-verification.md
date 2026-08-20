# Verification

## Focused Checks

- `cd sdk/go && go test . -run 'TestAdminRuntimeTransportCreatesAndDeletesDeviceSessionThroughRuntime|TestAdminRuntimeTransportRejectsLegacyDeviceSessionAliases|TestAdminRuntimeTransportRunsHubAndPairingLifecycleThroughRuntime' -count=1`
  - Result: pass.
- `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_profile_bridge.py::DaemonProfileBridgeTests::test_admin_profile_dispatches_gateway_trust_and_sessions sdk/python/tests/test_profile_bridge.py::DaemonProfileBridgeTests::test_admin_profile_rejects_legacy_device_session_aliases`
  - Result: 2 passed.
- `cd sdk/go && go test . -run 'TestAdmin|TestGoMEMCExecutesSharedProfileExclusivityConformanceCase|TestGoExecutesSharedAdminGatewayCarrierStatusCase' -count=1`
  - Result: pass.
- `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_admin.py sdk/python/tests/test_profile_bridge.py`
  - Result: 18 passed.
- `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_conformance.py::SharedConformanceFixtureTests::test_python_admin_gateway_executes_shared_carrier_status_conformance_case sdk/python/tests/test_conformance.py::SharedConformanceFixtureTests::test_python_memc_executes_shared_profile_exclusivity_conformance_case`
  - Result: 2 passed.

## Guards

- `bash tools/scripts/check-sdk-scaffold.sh`
  - Result: `check-sdk-scaffold ok`.
- `rg -n 'agentUra|ownerUra|deviceUra|hubUra|sessionId|sessionKind|createdUnixMs|expiresUnixMs|tokenId|pairingToken|credentialId|verificationMethod|nextCursor' sdk/go/admin_runtime.go sdk/python/easynet_sdk/profile_bridge.py`
  - Result: no matches.

## Aggregate Audit

- `bash tools/scripts/check-sdk-completion-audit.sh`
  - Result: `SDK completion audit ok`.
  - Included passing scaffold, parity matrix, section 27 coverage, URA naming, daemon latest input boundary, product smokes, Python SDK live smoke, and Go SDK live smoke.

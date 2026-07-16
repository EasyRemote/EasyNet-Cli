# SDK Adapter Report Evidence Plan

## Goal

Converge SDK conformance adapter reports onto one runner-owned evidence proof path.

## Root fork

Adapter report JSON files carry SHA-256 digests for source evidence, while the existing scaffold gate only validates report shape. Without a repository gate for those digests, a report can be syntactically valid yet stale after source edits.

## Boundary decision

- `sdk/conformance/refresh_adapter_report_evidence.py` owns derived evidence digest refresh/check behavior.
- `tools/scripts/check-sdk-conformance-reports.sh` owns live runner execution and must fail before execution when committed report evidence is stale.
- `tools/scripts/check-sdk-scaffold.sh` owns required SDK scaffold inventory and must include the refresh tool as part of the conformance surface.

## Edit sequence

1. Add the refresh script to the SDK scaffold required artifact list.
2. Copy the refresh script in the scaffold self-test sandbox.
3. Call the refresh script in the conformance report gate before source snapshot and runner execution.
4. Verify refresh self-test, stale-check, scaffold tests, and architecture/project gates.

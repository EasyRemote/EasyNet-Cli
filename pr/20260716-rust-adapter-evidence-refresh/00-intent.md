# Rust Adapter Evidence Refresh

## Concrete Use Case

`check-sdk-cutover-readiness.sh` fails because the Rust adapter report still
records an old digest for the `daemon/permission_denied` evidence source. The
test file exists and remains the intended proof, but the report no longer binds
to the current source bytes.

## Owner Boundary

- `sdk/conformance/runner/*-action-adapter-report.json` owns static adapter
  evidence metadata.
- `sdk/conformance/refresh_adapter_report_evidence.py` owns digest refresh and
  path safety checks.
- Runtime behavior and tests remain unchanged in this slice.

## Public Behavior

No API or runtime behavior changes. The change restores the conformance report
proof chain so SDK cutover gates verify the current source state.

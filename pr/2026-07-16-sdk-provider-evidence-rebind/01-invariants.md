# Invariants

## Evidence binding

Every action-adapter report evidence row must bind to the exact current bytes of
its repository-local evidence file.

For this slice, every evidence entry in the Go and Python adapter reports must
match the current bytes of its `ref_path`. The stale entries are:

- `sdk/conformance/runner/go-action-adapter-report.json`
  `access_control/provider` must hash `sdk/go/access_control_test.go`.
- `sdk/conformance/runner/go-action-adapter-report.json`
  `receipt/history_provider` must hash `sdk/go/receipt_test.go`.
- `sdk/conformance/runner/go-action-adapter-report.json`
  `runtime/administration_seam` must hash `sdk/go/runtime_admin_test.go`.
- `sdk/conformance/runner/python-action-adapter-report.json`
  `access_control/provider` must hash
  `sdk/python/tests/test_access_control.py`.
- `sdk/conformance/runner/python-action-adapter-report.json`
  `receipt/history_provider` must hash `sdk/python/tests/test_receipt.py`.
- `sdk/conformance/runner/python-action-adapter-report.json`
  `runtime/administration_seam` must hash
  `sdk/python/tests/test_runtime_admin.py`.

## State boundary

This is an evidence rebind, not a provider proof promotion. The canonical public
API model may still classify the access-control capability as `seam` until a
separate provider-proof bundle closes the provider-backed requirements.

## Verification boundary

The authoritative executable gate is `check-sdk-conformance-reports.sh` for the
Go/Python language slice, because that gate recomputes evidence hashes from the
snapshot source tree before executing adapter selectors.

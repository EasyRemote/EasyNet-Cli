# Verification

Completed checks:

- `cargo test admin_gateway --lib`
- `cargo test feature_discovery --lib`
- `go test ./...` from `sdk/go`
- `PYTHONPATH=sdk/python /opt/anaconda3/bin/pytest sdk/python/tests/test_cabi.py sdk/python/tests/test_conformance.py`
- `cargo fmt --check`
- `git diff --check`
- Targeted scan for retired address terminology in the touched Admin/Gateway
  files returned no matches.

Decision notes:

- Device revoke now has an SDK-owned carrier and projection through Rust/C ABI,
  Go, and Python.
- The carrier still delegates execution to Runtime Core invoke; the SDK does
  not own Hub trust state or backend account/session state.
- Hub join/leave, pairing lifecycle, credential verification, and device-session
  create/delete remain explicit daemon/ABI gaps for later slices.

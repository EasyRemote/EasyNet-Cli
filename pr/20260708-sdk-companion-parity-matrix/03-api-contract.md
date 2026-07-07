# API Contract

- Add `runtime_companion_control` to the Go/Python SDK parity matrix.
- Mark it `provider-backed` for both languages.
- Attach Go evidence to `sdk/go/daemon_test.go` and
  `sdk/go/conformance_test.go`.
- Attach Python evidence to `sdk/python/tests/test_daemon.py`,
  `sdk/python/tests/test_cabi.py`, and `sdk/python/tests/test_conformance.py`.
- Add a shared conformance case that requires DTO validation and lifecycle
  action projection.

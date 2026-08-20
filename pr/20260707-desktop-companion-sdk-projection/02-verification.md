# Desktop Companion SDK Projection Verification

## Commands

```text
PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_daemon.py
PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_daemon.py sdk/python/tests/test_cabi.py -k 'companion or daemon_handle_exposes_desktop_companion_lifecycle or daemon_transport_exposes_desktop_companion_lifecycle'
python3 -m py_compile sdk/python/easynet_sdk/companion.py sdk/python/easynet_sdk/daemon.py sdk/python/easynet_sdk/_cabi.py sdk/python/easynet_sdk/__init__.py
go test . -run 'TestDaemon'
git diff --check
```

## Evidence

- Python daemon facade tests pass.
- Python fake C ABI companion lifecycle test passes.
- Go daemon tests pass from `sdk/go`.
- Whitespace check passes.

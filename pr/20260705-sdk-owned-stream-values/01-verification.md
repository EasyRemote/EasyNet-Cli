# SDK-Owned EasyRemote Stream Values Verification

## Gates

- `PYTHONPATH=sdk/python python -m ruff check sdk/python/easynet_sdk/transport.py sdk/python/tests/test_transport.py`
- `PYTHONPATH=sdk/python python -m mypy sdk/python/easynet_sdk/transport.py`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_transport.py -q`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests -q`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. python -m ruff check easyremote tests`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. python -m mypy easyremote`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. python -m pytest -q`
- `audit_easyremote_cutover('/Users/macbook.silan.tech/Documents/GitHub/EasyRemote')`
- The daemon SDK requirements spec remains unchanged.

## Results

- SDK focused ruff: passed.
- SDK transport mypy: passed for 1 source file.
- SDK transport tests: 24 passed.
- Full Python SDK tests: 372 passed.
- `check-sdk-scaffold.sh`: passed.
- EasyRemote ruff: passed.
- EasyRemote mypy: passed for 27 source files.
- EasyRemote tests: 293 passed, 4 skipped, with the existing lambda-schema
  warnings in `tests/test_node.py`.
- EasyRemote cutover audit: ok, 0 violations.

## Coverage

- SDK transport tests cover stream value projection, JSON null, bytes payloads,
  clean terminal frames, envelope errors, host-stream error payloads, and idle
  timeouts.
- EasyRemote stream tests continue to prove public iteration and exception
  behavior after delegating projection to SDK.

## Remaining Work

- Schema-backed stream terminal receipt events remain Runtime Core work.
- Bidi product projection remains separate from this stream-value slice.

# Runtime-Owned EasyRemote Unary Wait Verification

## Gates

- `PYTHONPATH=sdk/python ruff check sdk/python/easynet_sdk/transport.py sdk/python/tests/test_transport.py`
- `PYTHONPATH=sdk/python python -m mypy sdk/python/easynet_sdk/transport.py`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_transport.py`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. python -m ruff check easyremote tests`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. python -m mypy easyremote`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. python -m pytest -q`
- `audit_easyremote_cutover('/Users/macbook.silan.tech/Documents/GitHub/EasyRemote')`
- The daemon SDK requirements spec remains unchanged.

## Results

- `PYTHONPATH=sdk/python ruff check sdk/python/easynet_sdk/transport.py sdk/python/tests/test_transport.py`:
  passed.
- `PYTHONPATH=sdk/python python -m mypy sdk/python/easynet_sdk/transport.py`:
  passed.
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_transport.py`:
  17 passed.
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests`:
  365 passed.
- `bash tools/scripts/check-sdk-scaffold.sh`: passed.
- EasyRemote ruff: passed.
- EasyRemote mypy: passed for 27 source files.
- EasyRemote tests: 293 passed, 4 skipped, with the existing lambda-schema
  warnings in `tests/test_node.py`.
- EasyRemote cutover audit: ok, 0 violations.

## Coverage

- Timed-out active calls retire their transport and close it after daemon return.
- Queued wait timeouts do not retire the active transport.
- Active close is bounded and retires the current owned transport.
- Idle close releases the current owned transport and permits a fresh owned
  transport on the next invoke.
- Externally-owned transports are not closed by the pool.

## Remaining Work

- Stream/bidi live-tail and terminal event conformance remain separate Runtime
  Core work.
- Full EasyRemote AgentControl/Server and publication product extraction remain
  separate product cutover slices.

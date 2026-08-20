# Verification

Executed checks:

- `PYTHONPATH=sdk/python python -m ruff check sdk/python/easynet_sdk/transport.py sdk/python/tests/test_transport.py`
  - passed
- `PYTHONPATH=sdk/python python -m mypy sdk/python/easynet_sdk/transport.py`
  - passed
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_transport.py -q`
  - `30 passed`
  - Covers open-session close compensation and unrelated invalid-argument
    propagation without synthetic cancel.
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. python -m ruff check easyremote/client.py easyremote/_sdk_transport/__init__.py tests/test_client.py`
  - passed
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. python -m mypy easyremote`
  - passed
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. python -m pytest tests/test_client.py -q`
  - `64 passed`
- Full SDK and EasyRemote test gates after focused checks pass.
  - SDK: `386 passed`
  - `check-sdk-scaffold ok`
  - EasyRemote ruff: passed
  - EasyRemote mypy: passed
  - EasyRemote: `273 passed, 4 skipped`
  - EasyRemote cutover audit: `ok=True violations=0`

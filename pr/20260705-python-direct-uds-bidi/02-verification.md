# Verification

Executed commands:

- `ruff check sdk/python` — passed.
- `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests/test_direct_runtime.py -q` — 13 passed.
- `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests/test_direct_runtime.py sdk/python/tests/test_bidi.py sdk/python/tests/test_runtime.py -q` — 38 passed.
- `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests -q` — 422 passed.
- `PYTHONPATH=sdk/python python -m mypy sdk/python/easynet_sdk/direct_runtime.py` — passed.
- `PYTHONPATH=sdk/python:sdk/python/tests python -m mypy sdk/python/tests/test_direct_runtime.py --ignore-missing-imports` — passed.
- `bash tools/scripts/check-sdk-scaffold.sh` — passed.

Follow-up typed-target correction:

- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_direct_runtime.py -q` — 13 passed.
- `PYTHONPATH=sdk/python python -m ruff check sdk/python/easynet_sdk/direct_runtime.py sdk/python/tests/test_direct_runtime.py` — passed.
- `PYTHONPATH=sdk/python python -m mypy sdk/python/easynet_sdk/direct_runtime.py sdk/python/tests/test_direct_runtime.py --ignore-missing-imports` — passed.
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests -q` — 422 passed.
- `PYTHONPATH=sdk/python python -m ruff check sdk/python` — passed.
- `bash tools/scripts/check-sdk-scaffold.sh` — passed.

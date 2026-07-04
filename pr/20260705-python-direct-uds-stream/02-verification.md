# Verification

Focused gates:

```sh
PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_direct_runtime.py -q
PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_direct_runtime.py sdk/python/tests/test_runtime.py sdk/python/tests/test_stream.py sdk/python/tests/test_transport.py -q
PYTHONPATH=sdk/python python -m ruff check sdk/python/easynet_sdk/direct_runtime.py sdk/python/tests/test_direct_runtime.py
PYTHONPATH=sdk/python python -m mypy sdk/python/easynet_sdk/direct_runtime.py sdk/python/tests/test_direct_runtime.py --ignore-missing-imports
```

Result: passed.

Broader gates:

```sh
PYTHONPATH=sdk/python python -m pytest sdk/python/tests -q
PYTHONPATH=sdk/python python -m ruff check sdk/python
bash tools/scripts/check-sdk-scaffold.sh
```

Result: passed, including `419 passed` for the full Python SDK test suite.

Boundary note: the direct stream adapter keeps Axon `InvokeStreamChunk` private
and projects its zero-based wire sequence into the positive SDK `StreamEvent`
sequence required by `StreamHandle`, without changing ordering or terminal-state
semantics.

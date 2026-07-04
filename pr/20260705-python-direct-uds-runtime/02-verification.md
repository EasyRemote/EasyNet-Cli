# Verification

## Focused gates

```sh
PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_direct_runtime.py -q
PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_runtime.py sdk/python/tests/test_connection.py sdk/python/tests/test_transport.py -q
PYTHONPATH=sdk/python python -m ruff check sdk/python/easynet_sdk/direct_runtime.py sdk/python/easynet_sdk/transport.py sdk/python/tests/test_direct_runtime.py
PYTHONPATH=sdk/python python -m mypy sdk/python/easynet_sdk/direct_runtime.py sdk/python/easynet_sdk/transport.py --ignore-missing-imports
PYTHONPATH=sdk/python python -m mypy sdk/python/tests/test_direct_runtime.py --ignore-missing-imports
```

Result: passed.

## Broader gates

```sh
PYTHONPATH=sdk/python python -m pytest sdk/python/tests -q
PYTHONPATH=sdk/python python -m ruff check sdk/python
bash tools/scripts/check-sdk-scaffold.sh
```

Result: passed.

## Known type-check boundary

`PYTHONPATH=sdk/python python -m mypy sdk/python/easynet_sdk/direct_runtime.py sdk/python/easynet_sdk/transport.py sdk/python/easynet_sdk/environment.py sdk/python/tests/test_direct_runtime.py`
still reports existing `SdkEnvironment` profile-client protocol typing debt
outside this transport slice, plus missing third-party `grpc` stubs when
`--ignore-missing-imports` is omitted. The new direct runtime source files and
tests pass mypy with missing third-party stubs ignored.

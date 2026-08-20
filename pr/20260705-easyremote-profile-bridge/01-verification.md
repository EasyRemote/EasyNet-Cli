# SDK-Owned EasyRemote Profile Bridge Verification

## Gates

- `PYTHONPATH=sdk/python ruff check sdk/python/easynet_sdk/easyremote_profiles.py sdk/python/tests/test_easyremote_profiles.py`
- `PYTHONPATH=sdk/python python -m mypy sdk/python/easynet_sdk/easyremote_profiles.py`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_easyremote_profiles.py`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests`
- EasyRemote focused tests after replacing product-local profile glue.

## Results

- `PYTHONPATH=sdk/python ruff check sdk/python/easynet_sdk/easyremote_profiles.py sdk/python/tests/test_easyremote_profiles.py`:
  passed.
- `PYTHONPATH=sdk/python python -m mypy sdk/python/easynet_sdk/easyremote_profiles.py`:
  passed.
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_easyremote_profiles.py`:
  5 passed.
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests`:
  377 passed.
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. ruff check easyremote tests`:
  passed in EasyRemote.
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. mypy easyremote`:
  passed in EasyRemote, 27 source files.
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. python -m pytest tests/test_sdk_profiles.py tests/test_control.py tests/test_mission.py tests/test_pipeline.py`:
  33 passed in EasyRemote.
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. python -m pytest`:
  272 passed, 4 skipped in EasyRemote.
- `PYTHONPATH=sdk/python python -m easynet_sdk.cutover_audit /Users/macbook.silan.tech/Documents/GitHub/EasyRemote`:
  passed with only the known `runpy` module-cache warning.

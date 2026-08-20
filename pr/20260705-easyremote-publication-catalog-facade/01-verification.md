# Verification

## Focused Gates

- `PYTHONPATH=sdk/python ruff check sdk/python/easynet_sdk/publication.py sdk/python/easynet_sdk/__init__.py sdk/python/tests/test_publication.py`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_publication.py`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. ruff check easyremote/control.py tests/test_control.py`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. python -m pytest tests/test_control.py`

## Full Gates

- `PYTHONPATH=sdk/python python -m mypy sdk/python/easynet_sdk/publication.py`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. mypy easyremote`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. python -m pytest`
- `PYTHONPATH=sdk/python python -m easynet_sdk.cutover_audit /Users/macbook.silan.tech/Documents/GitHub/EasyRemote`

## Expected Boundary Result

EasyRemote must not own ability catalogue filtering/show/list-device/list-user
semantics after this slice. Those rules live behind SDK publication facade
objects and continue to delegate URA validation through the injected SDK-backed
addressing facade.

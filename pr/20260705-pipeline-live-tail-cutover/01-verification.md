# Verification

## Focused Gates

- `PYTHONPATH=sdk/python python -m ruff check sdk/python/easynet_sdk/mission.py sdk/python/easynet_sdk/__init__.py sdk/python/tests/test_mission.py`
- `PYTHONPATH=sdk/python python -m mypy sdk/python/easynet_sdk/mission.py`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_mission.py -q`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. python -m ruff check easyremote/mission.py easyremote/_sdk_profiles.py tests/test_pipeline.py`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. python -m pytest tests/test_mission.py tests/test_pipeline.py tests/test_sdk_profiles.py -q`

## Full Gates

- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. python -m ruff check easyremote tests`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. python -m mypy easyremote`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:. python -m pytest`
- `PYTHONPATH=sdk/python python -m easynet_sdk.cutover_audit /Users/macbook.silan.tech/Documents/GitHub/EasyRemote`

## Boundary Result

Pipeline live event tailing is SDK-owned through `MissionEventTailer` and
`EasyRemoteMissionEventTailer`. EasyRemote exposes only product methods on
`MissionControl` and `MissionRun`; it does not manage event cursor advancement,
drop detection, or terminal shutdown.

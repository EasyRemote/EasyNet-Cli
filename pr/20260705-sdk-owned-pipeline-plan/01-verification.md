# Verification

## Focused

- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_mission.py -q`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python python -m pytest tests/test_pipeline.py -q`

## Broader gates

- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests -q`
- `bash tools/scripts/check-sdk-scaffold.sh`
- EasyRemote full lint/type/test suite after product facade migration.

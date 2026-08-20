# Mission Event Page Verification

## Gates

- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_conformance.py`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_mission.py`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests`

## Results

- `PYTHONPATH=sdk/python ruff check sdk/python/tests/test_conformance.py`:
  passed.
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_conformance.py sdk/python/tests/test_mission.py`:
  26 passed.
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests`:
  363 passed.

## Acceptance

- `sdk/conformance/cases/mission-carrier-status.yaml` lists mission event-page
  projection as an executed action, not scaffold-only.
- Shared fixtures include the event-list request and event-page response.
- Python conformance asserts event cursor monotonicity, ordered events, and
  terminal event projection through `MissionClient.events()`.

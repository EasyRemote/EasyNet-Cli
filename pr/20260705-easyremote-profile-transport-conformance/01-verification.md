# EasyRemote Profile Transport Verification

## Gates

- `ruff check easyremote tests`
- `mypy easyremote`
- `python -m pytest`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests`
- Focused tests assert unsupported SDK profile methods raise typed
  `NOT_IMPLEMENTED` SDK errors instead of falling through to raw transports.

## Result

- EasyNet-Cli Python SDK tests: `360 passed`.
- EasyRemote quality gate: ruff passed, mypy passed for 27 source files.
- EasyRemote tests: `293 passed, 4 skipped`, with the existing lambda-schema
  warnings in `tests/test_node.py`.
- EasyRemote Mission now consumes SDK-projected `mission.events` pages through
  `MissionControl.events()` and `MissionRun.events()`.

## Remaining Work

- Full AgentControl/Server product cutover remains separate from structural
  profile transport conformance.
- Backend/Hub Admin, session, pairing, and mission event routes still belong to
  the Go/backend SDK cutover work.

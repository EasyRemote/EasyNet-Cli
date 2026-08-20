# Verification

Status: Passed.

Commands run:

```sh
bash tools/scripts/check-sdk-conformance-reports.sh # ok
go test ./... # ok from sdk/go
PYTHONPATH=sdk/python uv run pytest -q sdk/python/tests/test_conformance.py # ok, 24 passed
TMPDIR=/tmp bash tools/scripts/check-sdk-scaffold.sh # ok
bash tools/scripts/check-sdk-ura-naming.sh # ok
bash tools/scripts/check-sdk-package-metadata.sh # ok
git diff --check # ok
bash tools/scripts/check-sdk-completion-audit.sh # ok, includes EasyRemote/backend product smokes and Python/Go live daemon smokes
```

Notes:

- `go test ./sdk/go` from the repository root failed because this repository
  has no root Go module. The successful command was run from `sdk/go`.
- `uv run pytest -q sdk/python/tests/test_conformance.py` and
  `uv run pytest -q tests/test_conformance.py` failed without `PYTHONPATH`
  because those invocations did not install the local package. The successful
  targeted command used `PYTHONPATH=sdk/python`, matching the live-smoke script
  package discovery model.

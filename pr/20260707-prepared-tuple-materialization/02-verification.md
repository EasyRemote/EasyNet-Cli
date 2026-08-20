# Verification

All commands were run from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`
on 2026-07-07.

- `cd sdk/go && go test ./...`
  - Result: pass.
- `PYTHONPATH=sdk/python python3 -m pytest sdk/python/tests/test_signing.py sdk/python/tests/test_conformance.py -q`
  - Result: pass, `43 passed`.
- `bash tools/scripts/python-sdk-live-smoke.sh`
  - Result: pass.
- `bash tools/scripts/go-sdk-live-smoke.sh`
  - Result: pass.
- `bash tools/scripts/check-sdk-completion-audit.sh`
  - Result: pass.
- `git diff --check`
  - Result: pass.

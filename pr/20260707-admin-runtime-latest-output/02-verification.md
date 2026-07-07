# Verification

All commands were run from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`
on 2026-07-07.

- `cd sdk/go && go test . -run 'TestAdminRuntimeTransport|TestGoMEMCExecutesSharedProfileExclusivityConformanceCase' -count=1`
  - Result: pass.
- `cd sdk/go && go test ./...`
  - Result: pass.
- `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_profile_bridge.py`
  - Result: pass.
- `TMPDIR=/tmp bash tools/scripts/check-sdk-scaffold.sh`
  - Result: pass.
- `bash tools/scripts/check-sdk-completion-audit.sh`
  - Result: pass after clearing stale generated `target/java-sdk-seam` build output.
- `git diff --check`
  - Result: pass.

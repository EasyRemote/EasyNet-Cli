# Verification

Passed:

- `PYTHONPATH=sdk/python:../EasyNet-Axon/sdk/python python3 -m pytest sdk/python/tests/test_authorized_runtime_session.py sdk/python/tests/test_runtime_ability.py`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph index .`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "_session_authority_admits_subject" --limit 20`
- `rg -n "def _session_authority_admits_subject|_session_authority_admits_subject\\(" sdk/python/easynet_sdk sdk/python/tests tools/scripts -S`
- `rg -n "session_authority_admits_subject\\(authority, subject_ura\\)|session_authority_admits_subject\\(authority, intent\\.subject\\.ura\\)" sdk/python/easynet_sdk/authorized_runtime_session.py tools/scripts/check-canonical-runtime-convergence-v2.sh tools/scripts/check-architecture-convergence.sh -S`

Notes:

- Direct `python3 -m pytest ...` failed before `PYTHONPATH` was set because the
  package root was not importable.
- `PYTHONPATH=sdk/python python3 -m pytest ...` then failed because the Axon
  Python SDK dependency was not importable.
- The repository script convention uses
  `PYTHONPATH=sdk/python:../EasyNet-Axon/sdk/python`; that command passed.
- `codegraph query "_session_authority_admits_subject"` returned only the
  canonical Rust/Python helper names and imports; it did not find the removed
  authorized-session wrapper.
- The residual `rg` wrapper check returned matches only in gate rejection
  rules, not in Python SDK implementation.

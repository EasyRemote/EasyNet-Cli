# Verification

Planned commands:

- `cd sdk/go && go test ./...` — passed.
- `PYTHONPATH=sdk/python:../EasyNet-Axon/sdk/python python -m pytest sdk/python/tests/test_authorized_runtime_session.py sdk/python/tests/test_runtime.py sdk/python/tests/test_ability_invocation.py sdk/python/tests/test_signing.py` — passed.
- `node --test sdk/node/test/runtime-core.test.mjs` — passed.
- `tools/scripts/check-java-sdk-seam.sh` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test` — passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync` — passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph status` — index up to date.
- `/Users/macbook.silan.tech/.local/bin/codegraph explore PreparedInvocation descriptor_ref signing_material fallback` — reviewed final blast radius.

Results will be recorded after execution.

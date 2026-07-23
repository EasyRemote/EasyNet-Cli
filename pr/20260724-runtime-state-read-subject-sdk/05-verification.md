# Verification

Completed:

- `cd sdk/go && go test . -run 'Test(RuntimeStateReadSubjectURA|AuthorizedRuntimeSessionHistoryAllowsUserOwnedResourceSubject|AuthorizedRuntimeSessionHistoryRejectsAuthoritySubjectMismatch|AuthorizedRuntimeSessionHistoryRejectsAllZeroSubject)'`
- `PYTHONPATH=sdk/python:../EasyNet-Axon/sdk/python python -m pytest sdk/python/tests/test_authorized_runtime_session.py -q`
- `node --test sdk/node/test/runtime-core.test.mjs`
- `python sdk/conformance/rebuild_public_api_model.py --write`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo fmt --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-runtime-state-read-subject-boundary.sh`
- `git diff --check`

Result: all passed.

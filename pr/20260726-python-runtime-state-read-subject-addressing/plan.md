# Python runtime-state read subject addressing convergence

## Goal

Remove the Python SDK runtime-state read subject helper's raw URA string construction and make it consume the canonical Addressing facade.

## Invariants

1. Python SDK runtime-state read subjects remain product-neutral runtime concepts.
2. URA grammar/building stays in the Axon-backed Addressing facade, not in `_session_authority_subjects.py`.
3. All-zero and missing user facts still fail before receipt-history provider dispatch.
4. Go/Python capability behavior remains aligned: both use canonical URA builders and structured URA projection, not substring parsing.
5. Public API names and return values remain compatible.

## Boundary proof

- Lower layer: `easynet_sdk.axon_addressing` wraps Axon SDK URA construction and parsing.
- Runtime subject helper: `_session_authority_subjects.runtime_state_read_subject_ura` validates runtime-specific preconditions, then delegates URA construction to `user_ura` + `resource_ura`.
- Receipt/history guard: continues consuming `is_runtime_state_read_subject_ura` and `session_authority_admits_subject`.

## Implementation checklist

1. Replace hand-written `easynet:///r/.../resource/user...` construction with canonical addressing helpers.
2. Keep existing all-zero/missing field behavior and tests.
3. Update convergence gate to require Addressing helper usage and reject raw runtime-state read subject string construction.
4. Run targeted Python tests and convergence gates.

## Decisions

- This change does not remove `runtime_state_read_subject_ura`; it is the public SDK helper. It removes its ownership of URA grammar.
- `parse_ura` remains in the predicate path because predicate validation consumes arbitrary caller input and must project structured owner/path facts.

## Verification

- `cd sdk/python && uv run pytest tests/test_authorized_runtime_session.py::AuthorizedRuntimeSessionTests::test_runtime_state_read_subject_ura_builds_user_owned_resource_subject tests/test_authorized_runtime_session.py::AuthorizedRuntimeSessionTests::test_runtime_state_read_subject_ura_rejects_all_zero_user_before_device_fallback tests/test_receipt.py -q`
- `python -m py_compile sdk/python/easynet_sdk/_session_authority_subjects.py`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`

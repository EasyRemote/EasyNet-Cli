# Python session-authority principal parity

## Goal

Remove the Python SDK request-side implicit promotion from `creator_principal_id` to `creator_principal_ura` so Python matches the Go SDK's explicit canonical principal field semantics.

## Invariants

1. `creator_principal_ura` remains the canonical staged public API field for canonical creator principal URA facts.
2. Request construction must not infer canonical principal fields from legacy/current wire fields.
3. Current daemon wire compatibility is preserved: explicit `creator_principal_ura` still lowers to `creator_principal_id` for the staged wire.
4. Response/metadata projection may still expose `creator_principal_ura` when the signed payload actually contains a canonical URA.
5. Go and Python request normalization converge: both require explicit `creator_principal_ura` for canonical creator-principal projection.

## Boundary proof

- Python request normalization currently has a private compatibility branch: `elif creator_principal_id.startswith("easynet:///")`.
- Go request normalization does not have this branch; it only processes explicit `CreatorPrincipalURA`.
- Removing the Python branch eliminates language-specific behavior while preserving explicit public API use.

## Implementation checklist

1. Remove Python request-side implicit creator principal URA promotion.
2. Add a regression test proving `creator_principal_id` alone does not populate `creator_principal_ura`.
3. Update convergence gate to forbid the retired promotion branch.
4. Run targeted Python authority tests and convergence gates.

## Decisions

- Do not remove `creator_principal_ura`; it is part of the canonical public API matrix.
- Do not change metadata projection; it reflects signed daemon payload facts, not caller request inference.

## Verification

- `cd sdk/python && uv run pytest tests/test_authority.py::AuthorityTests::test_session_authority_request_requires_explicit_creator_principal_ura tests/test_authority.py::AuthorityTests::test_authority_client_projects_canonical_principal_uras_to_current_session_wire tests/test_authority.py::AuthorityTests::test_authority_client_mints_session_through_transport -q`
- `python -m py_compile sdk/python/easynet_sdk/authority.py sdk/python/tests/test_authority.py`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`

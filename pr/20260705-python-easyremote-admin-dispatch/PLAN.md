# Python EasyRemote Admin Dispatch Plan

## Goal

Complete the EasyRemote Admin + Gateway profile bridge required by `docs/spec/daemon-sdk-requirements-v1.md` without changing the spec or reintroducing product-owned daemon transport logic.

## Boundary Proof

- EasyRemote profile bridge remains a product-facing dispatcher adapter over SDK-owned Admin DTOs.
- Carrier-builder methods stay fail-closed because the EasyRemote dispatcher does not own canonical Invocation construction.
- Admin dispatch methods call daemon system abilities by SDK-owned symbols and project product results into Admin DTO JSON.
- Python DTO validation remains in `AdminClient`, so invalid projections fail before escaping the SDK facade.
- No retired address terminology is introduced in touched files.

## Implementation Slices

1. Extend `AdminSystemAbility` with gateway, hub, pairing, credential, session, and revoke daemon ability names.
2. Implement EasyRemote Admin transport methods for gateway status, session list/create/delete, hub join/leave, pairing preflight/create/validate, credential verify, and device revoke.
3. Add focused EasyRemote bridge tests covering dispatch payloads and projected DTOs.
4. Run targeted Python tests, then full SDK validation before commit.

## Verification

- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_easyremote_profiles.py`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_direct_runtime.py`
- `go test -count=1 ./...` in `sdk/go`
- `go test -count=1 -tags easynet_cabi ./...` in `sdk/go`
- `cargo fmt --check`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `git diff --check`
- retired terminology scan over touched files

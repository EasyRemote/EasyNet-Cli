## Verification

Passed:

```text
codegraph explore runtime_identity ErrRuntimeIdentity NotFound Unavailable legacy identity api RuntimeIdentity load_runtime_identity runtime_identity_projection_from_json node_id canonical provider alias
python sdk/conformance/rebuild_public_api_model.py --write
bash tools/scripts/check-sdk-canonical-public-api.sh --self-test
bash tools/scripts/check-sdk-canonical-public-api.sh
bash tools/scripts/check-sdk-product-neutrality.sh
go test ./...
python -m py_compile sdk/conformance/sdk_public_surface_policy.py sdk/conformance/rebuild_public_api_model.py sdk/conformance/sdk_concepts.py
bash tools/scripts/check-architecture-convergence.sh
codegraph sync . && codegraph status
git diff --check -- sdk/conformance/sdk_public_surface_policy.py sdk/conformance/canonical-public-api.json sdk/conformance/sdk-parity-matrix.json pr/20260716-runtime-identity-error-quarantine
```

Post-generation assertions:

- `ErrRuntimeIdentityNotFound` is absent from `languages.go`.
- `ErrRuntimeIdentityUnavailable` is absent from `languages.go`.
- both aliases are present under `non_canonical.languages.go`.
- both aliases have `legacy_quarantine` metadata pointing at
  `capability_inventory.runtime_identity`.
- neither alias appears in `sdk-parity-matrix.json`.

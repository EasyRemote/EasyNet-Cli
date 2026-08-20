Verification
============

Executed checks:

- `go test ./...` from `sdk/go` - passed
- `python3.12 sdk/conformance/rebuild_public_api_model.py --write` -
  completed
- `tools/scripts/check-sdk-canonical-public-api.sh` - passed
- `tools/scripts/check-sdk-parity-matrix.sh --self-test` - passed
- `tools/scripts/check-sdk-product-neutrality.sh` - passed
- `tools/scripts/check-sdk-ura-naming.sh` - passed
- `tools/scripts/check-architecture-convergence.sh` - passed
- `git diff --check` - passed

Generated model delta:

- `NewRuntimeAdminClient` signature hash changed because the constructor now
  accepts `RuntimeLifecycle` instead of concrete `*RuntimeHost`.

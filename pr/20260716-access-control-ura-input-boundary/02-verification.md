# Verification

Planned checks:

- `cargo test -q authority_binding --lib`
- `cargo test -q policy_mutations_reject_scalar_only_identity_boundaries --lib`
- `cargo test -q policy_read_boundaries_reject_scalar_only_owner_identity --lib`
- `go test ./... -run AccessControl` from `sdk/go`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_access_control.py -q`
- `tools/scripts/check-architecture-convergence.sh`
- `rg -n '"owner_user_id"|"principal_id"'` over descriptors and SDK provider
  lowering to distinguish rejected inputs from projections.

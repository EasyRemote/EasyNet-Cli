# Managed signing active subject signer

## Invariants

- SDK signer selection must match daemon rotation semantics.
- Multiple active projections during rotation are not a global uniqueness failure; selection must be deterministic and auditable.
- The SDK may resolve a signer capability by subject and purpose, but actual signing remains behind the runtime key service.

## Implementation plan

1. Add `ManagedSigningClient.active_signer_for_subject`.
2. Filter active managed keys by exact bound subject and purpose.
3. Select the lexicographically first key id to mirror daemon rotation selection.
4. Fail closed when no active signer or no signer policy reference exists.

## Verification

- `uv run pytest -q sdk/python/tests/test_managed_signing.py`

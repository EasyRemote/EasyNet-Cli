# Boundary proof

## Root abstraction problem

The canonical runtime model already distinguishes signer custody from route
resolution and ability existence. Some remote paths project missing caller
signers as `CALLER_SIGNER_UNAVAILABLE`, while the native RuntimeHandle path can
still surface daemon KeyService lookup failures through `InvalidInvocation`.
That makes product callers observe SDK/daemon implementation detail instead of
the canonical runtime failure state.

## Ownership

- The signer-custody decision belongs to the runtime invocation boundary.
- C ABI integer return codes remain ABI-stable.
- `runtime_last_error_json` owns precise canonical error projection for SDK
  sidecars.
- Product UI and product-specific adapters must not parse keyring text or infer
  ability absence from signer readiness failures.

## Invariants

- Missing signer material is an admission/caller-identity failure, not
  descriptor absence and not generic protocol failure.
- Native RuntimeHandle invocation must not expose `keyring entry not found`,
  `keyring rejected request`, or `self-identity:` in product-visible messages.
- Public Rust API shape remains compatible; no public enum variant is added for
  this internal projection slice.
- The C ABI integer stays `ERR_PERMISSION_DENIED`, while the typed error DTO
  uses `CALLER_SIGNER_UNAVAILABLE` at stage `caller_identity`.

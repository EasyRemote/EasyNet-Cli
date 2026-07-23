# SDK EasyNet provider identity alias removal

## Goal

Remove the EasyNet provider credential `node_id` compatibility alias from the SDK-side runtime identity projection adapter.

The canonical SDK already requires `runtime_instance_id`. The EasyNet provider adapter may translate the product credential fact `device_id` into that runtime fact, but it must not continue accepting the retired `node_id` identity model.

## Boundary proof

- Canonical SDK root remains product-neutral and still rejects `node_id`.
- EasyNet provider remains a product-owned adapter, but its translation is now one explicit mapping: `device_id -> runtime_instance_id`.
- There is no fallback order between `device_id` and `node_id`.
- Conflicting alias handling is removed because the alias itself is no longer admitted.
- Public canonical SDK interfaces do not change.

## Invariants

1. A daemon credential projection without `runtime_instance_id` or `device_id` fails closed.
2. A daemon credential projection containing only `node_id` fails closed.
3. A daemon credential projection containing `device_id` and `node_id` fails closed because `node_id` is retired, even if values match.
4. Go and Python provider behavior remains equivalent.
5. SPEC v2 gate detects any reintroduction of `node_id` in these provider projection helpers.

## Verification plan

- Go provider lifecycle tests.
- Python runtime environment tests.
- SDK product-neutrality and SPEC v2 gates.
- codegraph sync/status.

## Delta log

- Removed Go EasyNet provider fallback from `node_id` to runtime instance identity.
- Removed Python EasyNet provider fallback from `node_id` to runtime instance identity.
- Replaced alias-mapping tests with fail-closed retired-field tests in both SDK implementations.
- Added SPEC v2 structural gate and mutation self-test to reject alias reintroduction.
- Verified Go/Python focused tests, fmt, SPEC v2, SDK product-neutrality, architecture convergence, public API, and codegraph.

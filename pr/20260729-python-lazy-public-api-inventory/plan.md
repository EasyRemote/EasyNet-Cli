# Python Lazy Public API Inventory Plan

## Goal

Restore SDK canonical public API source attestation for the Python SDK lazy export surface without changing the runtime import architecture.

## Invariants

1. The Python SDK top-level package remains a lazy export surface so provider helper modules can import in minimal plugin runtimes without eagerly loading every runtime dependency.
2. Every `__all__` symbol must still resolve through an explicit owned SDK module and source symbol.
3. The public API inventory must treat `_EXPORT_MODULES` as the canonical lazy import map, not as a compatibility fallback.
4. Missing, malformed, duplicate, or non-string lazy export map entries must fail closed.

## Boundary Proof

- The SDK public surface remains product-neutral; no EasyNet/EasyRemote-specific import path is introduced.
- Inventory ownership stays in `sdk/conformance/public_api_inventory.py`.
- Runtime import behavior in `sdk/python/easynet_sdk/__init__.py` is preserved.

## Verification Plan

1. Run `sdk/conformance/public_api_inventory.py python`.
2. Run `tools/scripts/check-sdk-canonical-public-api.sh`.
3. Run Python SDK import-boundary tests.
4. Run SPEC v2 normal and self-test gates.
5. Run architecture gate and `cargo fmt --check`.

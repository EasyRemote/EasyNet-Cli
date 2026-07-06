# Execution Checklist

- [x] Remove Go compatibility legacy request type aliases and wrapper methods.
- [x] Update Go compatibility tests to assert latest method names only.
- [x] Remove Python root exports for short compatibility aliases and product-style lifecycle facades.
- [x] Update Python import-boundary tests to enforce latest-only exports.
- [x] Rename exported Go/Python compatibility transports to latest profile method names.
- [x] Run focused Go and Python tests.
- [x] Record verification results and decisions.
- [x] Remove legacy Runtime Core prepare-option fields from Go/Python.
- [x] Update signed convenience helpers to use latest local signing policy.
- [x] Run focused Go/Python Runtime Core and C ABI tests.

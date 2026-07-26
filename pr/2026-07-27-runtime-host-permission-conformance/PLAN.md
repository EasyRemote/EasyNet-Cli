# Runtime-host permission conformance naming convergence

## Goal

Remove daemon-owned vocabulary from the canonical SDK conformance matrix for permission-denied projection.

## Root abstraction problem

The conformance case named `daemon/permission_denied` describes a generic runtime-host/socket permission failure. In the canonical SDK model, this is a runtime-host provider condition, not an EasyNet daemon capability or product lifecycle state.

## Invariants

- Public SDK conformance case identifiers must describe generic runtime capabilities.
- Permission-denied projection remains behaviorally unchanged.
- Reports, execution manifest, canonical public API inventory, and parity matrix must refer to the same case id.
- The product-neutrality gate must reject future `daemon/` SDK conformance case ids.

## Verification

- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- focused `rg` proving no `daemon/permission_denied` remains in SDK conformance

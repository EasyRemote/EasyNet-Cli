# Companion SDK Convergence Plan

## Goal

Converge desktop companion lifecycle exposure across SDK implementations without changing the desktop companion plugin specification or product architecture direction.

## Scope

- Add Swift companion DTOs, state enums, transport seam, and client facade.
- Add Java companion DTOs, state enums, transport seam, and client facade.
- Bind Go C ABI companion lifecycle functions into the existing daemon transport provider.
- Verify all additions preserve the shared daemon companion contract and do not introduce product-specific SDK abstractions.

## Non-goals

- No alternate companion lifecycle model.
- No product-specific directory, receipt, or naming model.
- No compatibility branch for C ABI libraries missing required companion symbols.
- No Axon Invocation lifecycle ownership for desktop companion processes.

## Expected Capability Matrix

| SDK | Companion capability state |
| --- | --- |
| Python | Provider-backed |
| Go facade | Provider-backed through C ABI when built with `easynet_cabi` |
| Swift | Seam |
| Java | Seam |

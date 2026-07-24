# Architecture

## Root abstraction problem

After deleting Go/Python `provider/easynet` source packages, the conformance
model still allowed `easynet_provider` as a canonical package category. That
keeps a product-owned provider role inside the SDK architecture even when the
source package no longer exists.

## Target model

- `distribution_facade` identifies historical language package roots whose
  import names may still carry product branding for public compatibility.
- `provider_neutral_core` continues to identify source roots that must be free
  of product semantics and are scanned by the neutrality gate.
- Runtime providers live under `provider/runtime` or equivalent language paths.

## Boundary

This is a conformance/model refactor. Runtime ingress, route terminality, and
receipt proof-fact execution paths remain separate work items.

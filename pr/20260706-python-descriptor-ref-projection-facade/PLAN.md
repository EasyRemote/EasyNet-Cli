# Python DescriptorRef Projection Facade Plan

## Objective

Expose a package-level Python SDK DescriptorRef projection helper so EasyRemote
and other consumers can request full descriptor projections through the
EasyNet-Cli SDK facade instead of splitting `@` locally.

## Invariants

- The SPEC remains unchanged.
- Axon remains the grammar owner for AbilityDescriptorRef parsing,
  canonicalization, and ability URA extraction.
- The SDK helper delegates through the default `AddressingClient` and
  `DescriptorRefRequest`; it does not implement DescriptorRef grammar in Python.
- Consumer boundary audits must flag local `project_descriptor_ref` helpers
  outside SDK identity facade wrappers.
- Existing package-level helpers and client methods keep their public behavior.

## Implementation Steps

1. Add `easynet_sdk.project_descriptor_ref(descriptor_ref, ...)` returning
   `IdentityProjection` through the default SDK addressing facade.
2. Export the helper from the package root and `__all__`.
3. Extend default-environment tests to prove the helper reaches the C ABI
   identity projection path and returns descriptor/ability/version facts.
4. Extend consumer-boundary tests so SDK imports/facade wrappers are allowed and
   local raw projection helpers are rejected.
5. Update README/parity status text without changing the normative SPEC.
6. Run focused Python tests, full Python SDK tests, lint, scaffold, whitespace,
   and conformance runner checks.

## Boundary Proof

The helper is a facade over `AddressingClient.project_descriptor_ref`; the
request is serialized as `DescriptorRefRequest` and projected by the existing
daemon/C ABI/Axon-owned identity path. No Python code splits DescriptorRefs,
constructs descriptor versions, or derives owner/ability facts. This closes the
consumer-side reason to hand-roll DescriptorRef projection while preserving the
Axon -> CLI -> SDK ownership chain.

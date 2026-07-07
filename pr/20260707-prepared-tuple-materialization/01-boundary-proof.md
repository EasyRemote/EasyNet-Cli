# Boundary Proof

## Ownership

Prepared tuple materialization is a daemon SDK facade concern. The daemon/Axon
runtime owns digest and hash material; language facades own projection into
strict SDK DTOs. This change does not move signing, hashing, descriptor
resolution, or admission policy into Go or Python.

## Invariants

1. Public `InvocationDraft` JSON remains latest-only and rejects unknown
   invocation fields.
2. Normalization applies only while decoding `PreparedInvocation`.
3. Removed fields are daemon/Axon materialization facts, not user-supplied
   invocation tuple fields.
4. DescriptorRef equality between `tuple` and `signing_material` remains
   enforced after normalization.
5. No URI terminology, legacy aliases, or product-specific SDK abstractions are
   introduced.

## Rejected Designs

- Making `InvocationDraft` accept digest/hash fields: rejected because that
  would leak daemon materialization into public SDK input.
- Ignoring unknown prepared tuple fields broadly: rejected because only the
  known daemon materialization fields are safe to strip.

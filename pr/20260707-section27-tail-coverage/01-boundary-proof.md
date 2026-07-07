# Boundary Proof

## Ownership

Section 27 coverage is a daemon SDK conformance gate. It proves that shared
case IDs exist and are exercised by language conformance surfaces. It does not
move product policy into Runtime Core and does not claim backend or EasyRemote
cutover by itself.

## Invariants

1. Every Section 27 case ID must be represented in
   `sdk/conformance/spec-section27-coverage.json`.
2. Every `covered_by` case must point to an existing shared conformance case.
3. MEMC cases stay generic SDK architecture checks and do not mention EasyNet or
   EasyRemote product lifecycle as SDK abstractions.
4. DescriptorRef delegation remains Directory + Identity / Axon-owned; facades
   do not concatenate descriptor refs or split URAs by hand.
5. No URI terminology or legacy input aliases are introduced.

## Rejected Designs

- Treating existing MEMC tests as implicit coverage: rejected because Section 27
  is a normative enumerated list and the gate must track it explicitly.
- Adding product-specific semantic alignment cases: rejected because the SDK
  owns generic runtime concepts, not product route semantics.

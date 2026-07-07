# Decisions

1. Match Go/Python authority DTO semantics in Node.
2. Keep Node authority minting transport-injected and seam-level.
3. Do not modify the existing provider-backed authority conformance case to fit
   Node.
4. Validate ambiguous authority metadata in InvocationBuilder so admission
   metadata cannot silently carry two authority families.

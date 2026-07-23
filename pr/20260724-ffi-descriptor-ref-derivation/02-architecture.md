# Architecture

## Root abstraction problem

The descriptor surface already owns canonical descriptor-ref derivation, but the FFI runtime descriptor catalog duplicated part of that derivation by manually formatting and canonicalizing descriptor refs. That creates a second authority over hash/action/version composition.

## Target ownership

- `AbilityDescriptor`: canonical descriptor identity and descriptor-ref derivation.
- FFI runtime catalog: serialization boundary only.
- Descriptor resolver: lookup and miss classification only.

## Boundary proof

The FFI layer may validate the returned descriptor hash format because it serializes the hash as a public catalog fact. It may not independently build descriptor refs from ability/version/hash/action parts.

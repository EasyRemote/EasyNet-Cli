# Architecture

## Boundary Proof

Axon owns DescriptorRef grammar, Ability URA projection, and ability address
facts. The Python SDK direct runtime is a transport implementation and protobuf
projection layer; it is not an identity parser.

## Runtime Model

The direct runtime receives an SDK Invocation draft containing a DescriptorRef.
Before constructing Axon Invocation messages, it calls the injected identity
projection facade:

1. `ability_ura_from_descriptor_ref(descriptor_ref)`
2. `ability_address(ability_ura)`

The direct runtime then validates ownership against `draft.callee_ura` and uses
the returned `public_name` as the daemon call target.

## Ownership

- Identity profile: DescriptorRef projection, Ability URA projection, ability
  address facts.
- Direct runtime: UDS channel lifecycle, bounded stream/bidi state machines,
  Axon protobuf request construction.
- Environment/direct invocation facade: lifecycle wiring for owned identity
  projection facades.

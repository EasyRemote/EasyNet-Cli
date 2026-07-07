# Node DescriptorRef Delegation Boundary Proof

## Ownership

Axon owns DescriptorRef canonicalization and ability-identity derivation.
Node owns only the facade methods that pass payloads to the injected
Directory/Identity transport and validate that required carrier fields are
present.

## Call Paths

```text
IdentityClient.projectDescriptorRef(...)
  -> injected IdentityTransport.projectDescriptorRef(...)

IdentityClient.abilityURAFromDescriptorRef(...)
  -> IdentityClient.projectDescriptorRef(...)
  -> injected IdentityTransport.projectDescriptorRef(...)

ReceiptClient.buildFetchInvocation(...)
  -> receiptFetchRequest(...)
  -> injected ReceiptTransport.buildFetchInvocation(...)
```

## Rejected Designs

- Splitting `descriptor_ref` on `@`: rejected because it would duplicate Axon
  grammar.
- Constructing `ability_ura + "@" + version`: rejected in facade code; Node may
  only relay to `buildDescriptorRef` transport projection.
- Defaulting receipt fetch `descriptor_ref`: rejected because it hides the
  complete Invocation carrier.

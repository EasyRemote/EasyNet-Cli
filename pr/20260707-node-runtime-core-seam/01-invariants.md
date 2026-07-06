# Invariants

## Boundary Invariants

- Node Runtime Core owns only DTO construction and object lifecycle.
- Transports are injected provider seams; no daemon socket, C ABI, Axon, or
  product route code is embedded in the facade.
- DescriptorRef and URA validation remain delegated to Identity/Axon helpers in
  future profile work.

## State Invariants

- Client close is idempotent.
- Closed clients reject runtime operations.
- InvocationBuilder is consumed only after a build transition succeeds.
- Stream and bidi handles expose explicit close/cancel operations through their
  transports.

## Naming Invariants

- Public identifiers use URA terminology only.
- No legacy input aliases are accepted.

# SDK-Owned EasyRemote Profile Bridge

## Objective

Move EasyRemote Admin/Mission system-ability carrier/projection glue into the
Python daemon SDK. EasyRemote should pass only product dispatch capability
(`device_ura` and `invoke_system_ability`) while SDK profile clients own request
DTO validation, daemon system-ability payloads, and typed projections.

## Boundary

- EasyNet-Cli SDK owns Admin/Mission carrier bases, nonce generation,
  EasyRemote-facing profile transports, and response projection into SDK DTOs.
- EasyRemote owns product client ergonomics, decorators, Pipeline DSL, and
  error mapping from SDK errors to EasyRemote exceptions.
- SDK must not import EasyRemote and must not reimplement Axon URA semantics.

## Invariants

- The bridge dispatches only named daemon system abilities from SDK enums.
- Product responses are projected through existing SDK `AdminClient` and
  `MissionClient` DTO parsers before EasyRemote receives them.
- Unsupported SDK profile operations fail closed with typed SDK errors rather
  than silently falling back to product-local transports.
- Nonce material is generated inside the SDK bridge and remains visible in the
  carrier base.

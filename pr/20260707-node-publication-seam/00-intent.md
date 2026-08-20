# Node Publication Seam Intent

## Goal

Add a P1 Node/TypeScript Publication profile seam that follows the daemon SDK
SPEC without claiming provider-backed support.

## Scope

- Expose a generic `PublicationClient` over injected transports.
- Delegate ResourceRef construction, package validation, deploy/unpublish
  carrier construction, plugin install, published-ability read models, and
  AbilityImpl lifecycle operations to the transport.
- Return `InvocationDraft` for carrier-building methods.
- Keep all product-specific plugin policy, host process lifecycle, package
  generation, and daemon provider wiring outside Node.

## Out Of Scope

- No C ABI bridge.
- No daemon subprocess provider.
- No product-specific EasyNet or EasyRemote abstractions.
- No local ResourceRef URA construction.
- No local AbilityDescriptor or AbilityImpl grammar parser.

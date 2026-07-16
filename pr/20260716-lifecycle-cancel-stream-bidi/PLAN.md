# Stream/Bidi Lifecycle Cancellation Plan

## Goal

Make daemon session-dispatched stream and bidi invocations visible to the same
canonical `invocation.cancel` lifecycle authority already used by unary
dispatch. Transport EOF and local reader shutdown remain transport concerns;
canonical cancellation is routed through the registered runtime invocation
handle.

## Invariants

- The cancellation registry tracks invocation lifecycles, not a unary-only
  transport shape.
- Stream and bidi opens register the original descriptor-bound envelope and the
  Axon runtime handle before publishing active data delivery.
- Runtime terminal finalization marks the lifecycle terminal in the registry so
  later cancel commands are idempotent and capacity retention remains bounded.
- Transport EOF may stop local forwarding, but it must not be represented as a
  signed `invocation.cancel` command.

## Scope

- Rename daemon session dispatcher ownership from unary cancellation to
  invocation lifecycle cancellation.
- Register carrier-v1 stream and bidi runtime handles with the registry.
- Mark stream and bidi registry entries terminal after canonical finalization.
- Add focused regression coverage for stream and bidi lifecycle registration.

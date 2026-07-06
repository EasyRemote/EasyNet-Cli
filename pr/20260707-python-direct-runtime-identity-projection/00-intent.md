# Python Direct Runtime Identity Projection

## Goal

Converge Python direct daemon runtime with the shared SDK runtime model by
delegating DescriptorRef projection to the Identity profile before building
Axon Invocation frames.

## Scope

- Direct daemon unary invocation request projection.
- Direct daemon server-stream request projection.
- Direct daemon bidi open-frame projection.
- Direct connector/environment/direct transport wiring for the identity facade.
- Tests proving identity-backed projection and fail-closed behavior.

## Non-Goals

- No DescriptorRef grammar parsing in Python Runtime Core.
- No product-specific ability naming.
- No legacy fallback that treats a DescriptorRef as a daemon function name.

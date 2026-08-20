# Boundary Proof

## SDK-Owned

- Admin request validation and JSON serialization.
- Typed projection DTOs for gateway status, agent pages, lifecycle results,
  pairing/device credential objects, device sessions, and device admin results.
- Client lifecycle over an injected transport.
- TypeScript declarations for the same seam.

## Provider-Owned

- DescriptorRef selection for `agent.*`, `session.*`, and pairing governed
  daemon abilities.
- Daemon lifecycle status projection.
- Pairing/device-session execution and credential production.

## Product-Owned

- Browser sessions and backend account-device binding.
- Certificate authority policy and TLS provisioning UX.
- Pairing copy, onboarding flows, quota/rate limits, and public route shaping.

## Rejected Designs

- SDK-derived gateway readiness: rejected because readiness flags are daemon
  facts and must not be collapsed by Node.
- Backend session DTOs: rejected because browser sessions are product state.
- System agent lifecycle aliases: rejected because system-owned agents are not
  managed by this profile seam.

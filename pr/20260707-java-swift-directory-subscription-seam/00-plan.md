# Java and Swift Directory Subscription Seam Plan

## Goal

Move Java and Swift Directory + Identity facades to the shared directory
subscription model already defined by the SDK conformance fixtures.

## Scope

- Add Java directory subscription cursor, request, projection, carrier, and
  stream-handle client seams.
- Keep Swift directory subscription behavior aligned with the same fixture and
  conformance case.
- Require complete Invocation tuple fields for subscription carrier requests.
- Project snapshot/live directory subscription state into bounded DTOs.
- Open subscription streams through injected Runtime Core stream sources.

## Non-Goals

- No daemon or C ABI provider.
- No SDK-owned fan-out.
- No product-specific event delivery or backend SSE/WebSocket policy.
- No alternate directory polling fallback.

## Capability State

Java and Swift Directory subscription move to `seam`. Provider-backed and
cutover-ready states remain unsupported.

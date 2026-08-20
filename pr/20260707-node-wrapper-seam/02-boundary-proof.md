# Boundary Proof

## SDK-Owned

- Wrapper DTO validation and JSON serialization.
- Client lifecycle over an injected projection transport.
- TypeScript declarations for the same seam.

## Runtime-Owned

- Invocation execution.
- Stream and bidi session transport.
- Terminal/desktop/browser/media governed ability dispatch.

## Product-Owned

- HTTP/WebSocket route bridges.
- Browser authorization and public route shaping.
- Storage, media, terminal, and desktop UX policy.

## Rejected Designs

- SDK-only wrapper execution path: rejected because Runtime Core remains the
  canonical execution path.
- Product bridge helpers in Node: rejected because backend HTTP/WebSocket policy
  is product-owned.

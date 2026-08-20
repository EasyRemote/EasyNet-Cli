# Boundary Proof

## SDK-Owned

- Compatibility request validation and JSON serialization.
- Typed projection DTOs for OpenAI-compatible model/chat/file shapes.
- Client lifecycle over an injected transport.
- TypeScript declarations for the same seam.

## Provider-Owned

- DescriptorRef selection for `openai.*` governed daemon abilities.
- Chat/model/file ability execution and receipt production.
- Projection from daemon facts into OpenAI-compatible envelopes.

## Product-Owned

- Public HTTP routes.
- API key handling, auth, quota, billing, and rate limits.
- Multipart upload parsing and object storage policy.
- SSE/WebSocket fanout.

## Rejected Designs

- Provider nickname models such as `gpt-4o`: rejected because the SDK profile
  uses canonical Ability URAs as model identifiers.
- SDK-side DescriptorRef concatenation: rejected because identity/daemon
  providers own descriptor projection.
- SSE event fanout in Node: rejected because backend/browser compatibility
  delivery is product-owned.

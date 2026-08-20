# Java and Swift Stream/Bidi Lifecycle Plan

## Goal

Move Java and Swift Runtime Core stream/bidi lifecycle behavior to the shared
`stream_bidi/lifecycle_state` model.

## Scope

- Make bidi `closeSend` an explicit local send-side state transition.
- Keep receive-side `next` active after local half-close.
- Reject `send` after local half-close with typed cancellation.
- Preserve idempotent stream close and bounded terminal projections.
- Update Java/Swift conformance declarations and seam evidence.

## Non-Goals

- No daemon handle registry or cross-owner close simulation in the language seam.
- No provider-backed stream transport.
- No product-specific session lifecycle.

## Capability State

Java and Swift Runtime Core stream/bidi lifecycle move from partial seam evidence
to explicit `seam` coverage for the shared lifecycle-state case.

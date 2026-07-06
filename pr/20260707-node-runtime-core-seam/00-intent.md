# Node Runtime Core Seam

## Goal

Move the Node/TypeScript SDK root from placeholder-only to a Runtime Core seam
that projects the same canonical DTO and lifecycle model as Go and Python.

## Non-goals

- Do not add daemon transport implementation.
- Do not add product-specific helpers.
- Do not add profile clients outside Runtime Core.
- Do not parse URA or DescriptorRef grammar in Node Runtime Core.

## Acceptance Criteria

- Node exposes feature discovery, typed errors, Invocation draft construction,
  RuntimeClient invoke/prepare/submit/stream/bidi handle seams, and explicit
  close lifecycle over injected transports.
- TypeScript declarations match the JavaScript facade.
- Node tests cover tuple validation, transport delegation, typed error decoding,
  and close-state behavior.
- The SDK scaffold gate runs the Node seam tests.

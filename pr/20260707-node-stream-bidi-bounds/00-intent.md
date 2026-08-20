# Node Stream Bidi Bounds Intent

Implement the Node/TypeScript Runtime Core stream and bidi bounded-state seam
required by `docs/spec/daemon-sdk-requirements-v1.md`.

## Scope

- Add named Node constants for maximum retained stream events and bidi frames.
- Retain bounded, inspectable history on `StreamHandle` and `BidiSession`.
- Project overflow as a typed terminal SDK error object without unbounded
  buffering.
- Keep transport receive semantics delegated to the injected transport.
- Keep Node as a seam; do not claim provider-backed callback-queue overflow
  conformance unless the shared case semantics are fully covered.

## Out Of Scope

- No daemon transport provider.
- No C ABI callback queue implementation in Node.
- No daemon-side RESOURCE_EXHAUSTED wire mapping.
- No new stream or bidi protocol grammar.

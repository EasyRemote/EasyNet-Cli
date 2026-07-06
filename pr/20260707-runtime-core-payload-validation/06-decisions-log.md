# Decisions Log

## 2026-07-07

- Chose builder-bound validation because `InvocationDraft` is the shared Runtime Core object for unary, stream, bidi, and prepare/sign/submit.
- Kept the existing non-empty raw payload rule intact; this slice only rejects malformed base64.

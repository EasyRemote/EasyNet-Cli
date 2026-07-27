# API Contract

- No public symbol may silently synthesize invocation tuple fields.
- Public compatibility at the edge may preserve error shape, but cannot construct receipts, admission decisions, signatures, or descriptor proof facts outside the canonical runtime path.
- If a legacy input shape is accepted, it must be translated into a complete descriptor-bound request before canonical dispatch; otherwise it must be rejected deterministically.

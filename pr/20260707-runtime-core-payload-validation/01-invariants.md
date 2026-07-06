# Invariants

## Semantic Invariants

- `InvocationDraft` accepts exactly one payload carrier.
- `args` is the JSON carrier and `arguments_base64` is the raw-byte carrier.
- Raw-byte payloads must be valid base64 before they can enter Runtime Core.

## Boundary Invariants

- Validation lives in the shared draft inspection path in both Go and Python.
- Product facades must not reinterpret or repair malformed payload encodings.
- No product-specific payload rule is introduced.

## Boundedness Invariants

- Validation is a single strict base64 decode of the provided payload string.
- The check is deterministic and independent of daemon availability.

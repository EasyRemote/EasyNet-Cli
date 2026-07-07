# Invariants

1. Compatibility is a generic daemon SDK profile, not a product API gateway.
2. Node preserves complete Invocation carrier context for daemon-dispatched
   operations: `caller_ura`, `callee_ura`, `subject_ura`,
   `descriptor_version`, `nonce_base64`, and `causal_context`.
3. Chat `model` values must be canonical Ability URAs, not provider nicknames.
4. Unary chat completion rejects `stream: true`; stream chat completion forces
   `stream: true` before delegating.
5. File objects are projection DTOs over daemon/resource facts; Node does not
   own multipart upload or storage policy.
6. Product-owned HTTP auth, quota, billing, and SSE fanout must not enter the
   Node SDK seam.
7. No non-URA naming and no legacy input aliases are introduced.

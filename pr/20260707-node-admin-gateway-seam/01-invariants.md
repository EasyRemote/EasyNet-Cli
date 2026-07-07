# Invariants

1. Admin + Gateway is a generic daemon profile, not a backend onboarding layer.
2. Node preserves complete Invocation carrier context for daemon-dispatched
   operations: `caller_ura`, `callee_ura`, `subject_ura`,
   `descriptor_version`, `nonce_base64`, and `causal_context`.
3. Agent lifecycle targets reject system-reserved names.
4. GatewayStatus preserves daemon readiness flags and never derives product
   readiness.
5. Pairing and device-session DTOs are daemon projections; product token copy,
   authorization, and session UX stay outside the SDK.
6. No non-URA naming and no legacy input aliases are introduced.

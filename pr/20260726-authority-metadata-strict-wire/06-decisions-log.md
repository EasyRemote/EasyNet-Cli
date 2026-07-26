# Decisions Log

- 2026-07-26: Selected authority metadata strict parsing because codegraph showed session/delegation authority payloads and signed wire wrappers lacked fail-closed unknown-field behavior while downstream admission relies on these facts for subject, callee, audience, scopes, and expiry.
- 2026-07-26: Treat unknown fields as malformed authority metadata, not compatibility extensions. This preserves public metadata keys while removing legacy carrier tolerance.
- 2026-07-26: Applied `deny_unknown_fields` to both canonical authority payloads and both signed authority wire wrappers. Negative tests use retired carrier names without adding alternate identity terminology.

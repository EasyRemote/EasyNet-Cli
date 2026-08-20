# Invariants

- Every operation must lower to a complete Invocation tuple through Runtime
  Core; product calls must not use control frames.
- Descriptor refs must be resolved via `IdentityClient`; Go must not define a
  descriptor-ref grammar or concatenate `@version`.
- Hub membership results may project daemon output, but daemon remains the
  authority for trust state, credential state, and membership effects.
- Pairing and credential DTO projections must fail closed when daemon output is
  missing required device, hub, credential, token, expiry, or scope facts.
- Browser/backend account state and onboarding copy remain product concerns.

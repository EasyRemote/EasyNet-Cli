# API Contract

No public method signatures change.

Behavioral tightening:

- Generic provider-backed ability invocation rejects governance read abilities before descriptor resolution.
- The error directs callers to typed runtime providers instead of surfacing remote route/signer/admission failures.
- Existing typed `RuntimeAbilityClient` catalogue and receipt provider paths continue to send explicit descriptor provider fields.

# Session Authority Generic Model

## Intent

Converge session authority metadata on generic runtime authority facts instead of
product-specific backend, user, session, or audience-list fields.

The SDK owns typed projections and validation shape for runtime authority
metadata. Product repositories may decide how to obtain those facts, but the SDK
must only expose generic issuer, subject, audience, scope, and lifetime
concepts.


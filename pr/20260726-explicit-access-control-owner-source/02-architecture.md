# Architecture

`authority_binding.check` is a governance read over canonical runtime policy state.

The daemon owns deserialization and final domain validation. SDKs own typed request construction and must reject semantically incomplete calls before invoking the provider. Product code remains a consumer of SDK typed clients and must not rely on hand-built JSON defaults.

The root abstraction problem is an implicit owner-source inference at the daemon boundary, mirrored by SDK optional argument projection. The fix is to make `owner_source` an explicit part of the canonical check tuple.

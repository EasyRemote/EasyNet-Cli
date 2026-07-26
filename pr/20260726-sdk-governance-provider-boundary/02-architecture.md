# Architecture

The SDK has three semantic lanes:

1. Public action invocation: generic descriptor-bound invoke/stream/bidi.
2. Catalogue provider: `meta.list_abilities` through the ability descriptor provider.
3. Receipt provider: `invocation.history.*` and `invocation.trace.*` through the receipt history provider.

The descriptor resolver still accepts explicit providers, but generic action clients must not silently omit the provider for governance reads. Typed providers own the provider selection and subject policy.

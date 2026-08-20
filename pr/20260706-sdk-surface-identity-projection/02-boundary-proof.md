# Boundary Proof

`SurfaceRuntimeTransport` already composes `RuntimeClient` and `IdentityClient`.
Runtime owns ability invocation; Identity owns URA and DescriptorRef projection.
Surface owns only the DTO view over daemon `pages.*` ability results.

Therefore the correct dependency flow is:

```text
pages.* daemon output
  -> SurfaceRuntimeTransport projection
  -> IdentityClient.ResourceURA(owner_ura, path) when a resource ref is missing
  -> SurfacePageRecord DTO
```

This keeps language facade code from copying Axon resource URA grammar while
still allowing the Surface profile to produce complete records from legacy or
minimal daemon outputs.

# Architecture

The SDK root `ResourceNamespace` constants are canonical runtime resource
families exposed to SDK consumers. The internal helper that validates and
projects those namespaces should therefore be named as runtime state, not
product state.

This slice only renames internal Go helpers and comments:

- `productResourceNamespaces` -> `runtimeResourceNamespaces`
- `isProductResourceNamespace` -> `isRuntimeResourceNamespace`
- `productResourceURA` -> `runtimeResourceURA`
- `projectProductResourcePath` -> `projectRuntimeResourcePath`

The SPEC v2 gate is extended so the root SDK cannot reintroduce
`productResource*` internals.

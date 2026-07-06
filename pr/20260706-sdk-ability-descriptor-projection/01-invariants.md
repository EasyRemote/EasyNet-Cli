# Invariants

1. The SDK owns descriptor read-model projection; product repos own display interpretation of free-form metadata.
2. Projection is pure and deterministic: same map in, same projection out.
3. Missing descriptor fields produce zero values, not panics.
4. Nested descriptor maps are merged before top-level summary fields; top-level fields win.
5. Visibility and scope enforcement are not duplicated in SDK projections. Runtime admission has already filtered visible descriptors.
6. Only URA terminology is used for routable identities and addresses.

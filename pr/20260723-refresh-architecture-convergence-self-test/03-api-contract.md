# API Contract

No public runtime or SDK API changes.

The test contract changes only in one place: the retired FFI descriptor remote
probe caller-default fixture now expects the current
`R95_DESCRIPTOR_RESOLVER_BOUNDED_CATALOG` rule marker instead of the removed
older marker.

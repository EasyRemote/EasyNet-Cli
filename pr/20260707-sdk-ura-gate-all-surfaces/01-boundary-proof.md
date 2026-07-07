# Boundary Proof

## Ownership

URA terminology is a cross-SDK architecture invariant. The guard belongs in the
SDK cutover/readiness toolchain because all language facades and shared
conformance artifacts implement the same canonical runtime model.

## Scope Boundary

Generated Axon protobuf code and generated Python protobuf modules remain
excluded. The gate covers authored SDK/conformance surfaces where naming is a
repository architecture decision.

## Product Boundary

The gate does not add product-specific concepts. It prevents product or legacy
address naming from leaking into generic runtime SDK surfaces.

## No Compatibility Alias

The guard treats retired address terms as failures rather than accepted aliases.

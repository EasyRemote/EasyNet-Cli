# Decisions

## Complete Transport Contract

`HostBindingTransport` must provide the full host-stream codec/hash surface:
binding build, request decode, item/error/terminal frame encode, and output-hash
folding. Optional methods would create a facade that can instantiate but fail at
the first host-stream operation, which weakens the Host Binding seam.

## Error Encoding Order

Plain JavaScript `Error` values are normalized before generic object DTOs.
Otherwise `Error` instances look like objects but lack the typed SDK error
fields required by the Host Binding error DTO contract.

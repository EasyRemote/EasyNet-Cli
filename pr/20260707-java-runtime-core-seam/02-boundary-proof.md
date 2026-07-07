# Boundary Proof

Java Runtime Core is a language facade over the canonical daemon SDK model.

This slice adds no daemon transport implementation and no protocol algorithms.
The injected `RuntimeTransport` is a seam that lets future C ABI or daemon
providers supply behavior while Java owns only idiomatic object lifetime,
validation, and state projection.

The Java package uses URA naming and descriptor refs as opaque SDK strings. It
does not parse or canonicalize descriptor refs locally.

Feature discovery is intentionally generic: the seam reports SDK profile and
symbol availability, not Axon protobuf/provider booleans. Provider-specific
capabilities can be projected later through a provider adapter without changing
the public Runtime Core object model.

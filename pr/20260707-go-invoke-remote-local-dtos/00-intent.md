# Intent

Converge the Go SDK Axon bridge facades with the daemon SDK boundary by
removing public Axon type aliases while preserving Axon-owned wire validation.

The Go SDK should expose EasyNet-Cli SDK DTOs, not Axon public type aliases.
Conversion into Axon types remains internal because Axon owns invoke-remote wire
encoding, byte-array validation, origin-caller claim validation, down-frame
decoding, and authority metadata canonicalization.

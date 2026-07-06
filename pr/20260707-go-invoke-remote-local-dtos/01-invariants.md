# Invariants

1. The Go SDK must not expose public type aliases to Axon bridge DTOs.
2. Wire encoding, byte validation, frame decoding, and origin-caller validation
   still delegate to Axon helpers.
3. Authority metadata canonicalization still delegates to Axon helpers.
4. Constants that are protocol truth must remain delegated to Axon, not
   hard-coded as a second source.
5. Public Go struct names and function names remain source-compatible for SDK
   consumers that do not import Axon types.
6. The bridge stays URA-only and does not reintroduce legacy `ability` input.

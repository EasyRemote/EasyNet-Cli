# Architecture

## Boundary

The CLI join helper writes operator-visible realm-trust configuration after canonical pairing facts are available. It must materialize the same canonical trust row shape the daemon later reads.

## Layering

- Pairing facts provide URA and public-key inputs.
- `TrustedAgentUpsert` owns row materialization.
- The daemon trust-anchor reader remains the authority for runtime admission.

## Ownership

This change keeps configuration write logic in the CLI facade but removes role-specific legacy preservation from the shared upsert implementation.

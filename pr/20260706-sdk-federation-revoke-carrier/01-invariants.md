# Invariants

1. The helper only builds the daemon carrier payload for `federation.revoke`.
2. It does not parse, validate, or canonicalize URA grammar.
3. It must not import raw Axon packages, generated protobufs, C ABI handles, or
   daemon internals.
4. Backend can replace `axonsdk.FederationRevokePayload` with this SDK facade
   without changing payload keys.


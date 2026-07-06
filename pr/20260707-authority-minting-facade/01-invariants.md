# Authority Minting Invariants

1. Authority payload creation is delegated below the SDK facade. Go and Python
   SDK code may validate envelope shape and decode projections, but must not
   compute Axon canonical authority bytes locally.
2. A single Invocation may carry either `x-easynet-delegation` or
   `x-easynet-session-authority`, never both.
3. Delegation authority binds issuer, subject, caller, audience, scopes,
   issued time, expiry, and signature.
4. Session authority binds backend, user, session id, scopes, audiences,
   issued time, expiry, and signature.
5. Minting requests cannot carry private key material. Signer/key policy stays
   with daemon/Axon-owned or integration-owned signer providers.
6. Client lifecycle is closed and deterministic: a closed authority client
   rejects further minting calls.

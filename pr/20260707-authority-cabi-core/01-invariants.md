# Authority C ABI Core Invariants

1. Authority payload canonical bytes are produced by Rust daemon SDK core using
   the same canonical JSON helper accepted by daemon admission.
2. `DelegationProof` and `SessionAuthority` metadata keys remain mutually
   exclusive.
3. Request metadata cannot carry private key material across SDK or C ABI
   boundaries.
4. C ABI prepare functions return signing material only; they never sign.
5. C ABI materialize functions accept a signature and emit the metadata value
   admitted by daemon invocation policy.
6. Output DTOs include enough metadata for language SDKs to prove they did not
   compute canonical payload bytes locally.

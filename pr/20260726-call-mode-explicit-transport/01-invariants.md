Invariants
==========

- Descriptor transport mode is a canonical routing fact.
- A missing call mode must not be silently projected as RPC through Rust
  `Default`.
- Builders may still select RPC explicitly where that is the authored
  constructor contract.
- Stream and bidi capabilities must remain governed by explicit descriptor
  metadata, not presentation hints.
- Federation owner projection rows must carry callable summary facts explicitly;
  lossy rows without mode geometry must fail closed instead of decoding as RPC.

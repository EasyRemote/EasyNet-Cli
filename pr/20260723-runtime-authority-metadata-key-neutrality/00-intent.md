# Runtime Authority Metadata Key Neutrality

## Intent

Remove product-specific EasyNet naming from canonical runtime authority metadata keys.

Delegation and session-authority metadata are generic runtime concepts. The SDK and daemon admission path must not expose `x-easynet-*` as canonical key names. This slice cuts over the canonical wire keys without adding an old-key compatibility path.

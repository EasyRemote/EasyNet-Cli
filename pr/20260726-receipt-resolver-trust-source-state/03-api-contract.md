# API Contract

No public Rust signatures change.

Behavioral tightening:

- Malformed realm trust anchor state now appears in the receipt signer trust error instead of being hidden behind `realm trust anchor is empty or unavailable`.
- Missing or empty anchors remain non-successful for non-local signers, but the reason is explicit.

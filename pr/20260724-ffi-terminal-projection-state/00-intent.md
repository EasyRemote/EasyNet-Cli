# Intent

Remove the FFI stream/bidi reader compatibility seam where lifecycle control is recovered from callback JSON via `projection["terminal"].as_bool().unwrap_or(false)`.

The callback JSON is a public projection. It must not be the internal authority for stream/bidi lifecycle. The reader should consume an explicit projection object that carries both:

- the public JSON frame to deliver to SDK callers;
- the canonical terminal state derived from Axon receipts/protocol projection.


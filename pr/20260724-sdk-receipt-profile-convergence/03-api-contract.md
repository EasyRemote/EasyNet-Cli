# API Contract

No public symbols are added or removed.

Existing public constructors and decoding methods keep their signatures:

- Node: `RuntimeReceipt.fromObject`
- Swift: `RuntimeReceipt.init(_:)`
- Java: `RuntimeReceipt.fromMap`

Behavioral contract change:

- receipts using `axon-strict-v2` continue to parse;
- receipts using `axon-legacy-v1` fail validation;
- receipts using `opaque` as an entity profile fail validation.

This is intentional cutover behavior, not a compatibility break in the public
surface shape.

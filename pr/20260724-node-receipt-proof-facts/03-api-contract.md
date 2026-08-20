# API Contract

## Public API

- Keep `RuntimeReceipt.fromObject`, `RuntimeReceipt.rawProjection`, and invocation result parsing behavior compatible.
- Invalid receipts raise SDK runtime validation errors.

## Canonical proof API

- Proof payload and proof hash remain receipt fields.
- Authority binding projection is internal canonical validation material, not a new product API.

## Error behavior

- Malformed proof facts fail closed as `INVALID_ARGUMENT` runtime errors.
- Missing or mismatched proof facts must not be normalized or synthesized by the Node SDK.

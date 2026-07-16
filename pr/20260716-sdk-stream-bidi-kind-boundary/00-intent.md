# Intent

Remove the stream/bidi runtime callback alias that lets the SDK boundary carry
`event` when the canonical frame field is `kind`.

This slice closes one legacy wire-shape fork: Rust C ABI callback projection,
Go/Python provider adapters, and SDK domain decoders must agree on one public
frame DTO. The C ABI provider is the owner of callback JSON shape and emits
canonical Runtime Core fields directly:

- frame kind: `kind`
- stream payload content type: `payload_content_type`
- binary payload bytes: `payload_base64`
- terminal proof: `terminal_receipt`

Expected effect: architecture convergence. Provider code stops protecting the
retired callback vocabulary, while SDK adapters remain private transport
normalizers instead of compatibility repair layers.

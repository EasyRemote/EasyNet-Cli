# Intent

Remove the prepared-invocation fallback that lets SDK decoders treat
`request_id` as a substitute for `prepared_id`.

The C ABI and runtime signing path already use `prepared_id` as the opaque
prepared-handle identity. This slice aligns the public Go and Python signing
DTOs with that boundary while preserving `request_id` as correlation metadata.

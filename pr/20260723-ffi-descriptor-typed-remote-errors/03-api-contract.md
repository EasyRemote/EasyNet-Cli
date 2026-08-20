# API Contract

## ABI entry point

`easynet_runtime_resolve_descriptor_ref` keeps the same ABI and still projects
typed failures through `ErrorProjection`.

## Typed states

- Invalid request -> `INVALID_ARGUMENT`, stage `sdk`, retry `never`.
- Invalid catalog payload -> `INVALID_ARGUMENT`, stage `provider_payload`,
  retry `never`.
- Missing runtime owner -> `CALLER_IDENTITY_UNAVAILABLE`, stage
  `caller_identity`, retry `never`.
- Runtime offline -> `RUNTIME_OFFLINE`, stage `transport`, retry
  `after_backoff`.
- Missing caller signer -> `CALLER_SIGNER_UNAVAILABLE`, stage
  `caller_identity`, retry `never`.
- Remote owner offline -> `DESCRIPTOR_OWNER_OFFLINE`, stage `routing`, retry
  `after_backoff`.
- Descriptor absent -> `DESCRIPTOR_NOT_FOUND`, stage `routing`, retry `never`.

## Error source contract

String formatting may remain for human-readable details after typed state has
already been selected. It must not select the state.

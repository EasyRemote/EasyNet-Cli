# Runtime Core Validation Parity

## Goal

Align Go and Python Runtime Core Invocation DTO validation with the daemon SDK SPEC by rejecting malformed Invocation nonces at build/decode time.

## Non-goals

- Do not change public DTO field names or transport payload shapes.
- Do not introduce product-specific EasyNet or EasyRemote behavior.
- Do not move Axon canonicalization into language facades.

## Acceptance Criteria

- Go `InvocationBuilder.Build` and JSON draft decode reject non-base64 or non-16-byte `nonce_base64`.
- Python `InvocationBuilder.build` and JSON draft decode reject non-base64 or non-16-byte `nonce_base64`.
- Error behavior remains typed as SDK invalid argument.
- Existing Runtime Core prepare/sign/submit, stream, and bidi tests continue to pass.

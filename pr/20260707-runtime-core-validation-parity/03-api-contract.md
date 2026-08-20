# API Contract

## Request DTO

`InvocationDraft` continues to require:

- `caller_ura`
- `callee_ura`
- `descriptor_ref`
- `subject_ura`
- `nonce_base64`
- `causal_context`
- exactly one payload carrier: `args` or `arguments_base64`
- `content_type`

## Validation Contract

- `nonce_base64` must be a base64 string.
- The decoded nonce must be exactly 16 bytes.
- Invalid nonce input raises the language SDK's invalid-argument error.

## Compatibility Contract

No legacy nonce aliases are accepted. No new fallback path is introduced.

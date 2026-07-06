# API Contract

## Request DTO

`InvocationDraft` continues to support exactly one of:

- `args`
- `arguments_base64`

## Validation Contract

- `arguments_base64` must be a non-empty base64 string under the current public contract.
- Invalid raw payload encoding raises the language SDK's invalid-argument error.

## Compatibility Contract

No payload alias is accepted and no fallback decoding is introduced.

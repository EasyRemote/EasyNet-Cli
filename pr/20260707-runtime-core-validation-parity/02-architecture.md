# Architecture

## Root Abstraction Problem

`InvocationDraft` is the canonical immutable seven-tuple snapshot for Runtime Core. If nonce validity is deferred to signing or transport paths, unary, stream, bidi, and prepared submission can disagree on the same draft's validity.

## Target Architecture

- Keep nonce validation inside the `InvocationBuilder` inspection path used by both build and JSON decode.
- Use the same validation rule in Go and Python: strict base64 decode and decoded byte length equal to 16.
- Preserve all public APIs and transport schemas.

## Module Boundaries

- `sdk/go/invocation.go`: Go Runtime Core DTO construction.
- `sdk/python/easynet_sdk/invocation.py`: Python Runtime Core DTO construction.
- Tests live beside existing Invocation DTO tests in both languages.

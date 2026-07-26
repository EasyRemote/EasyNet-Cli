# Decisions Log

## 2026-07-26

- Treat raw remote error text as diagnostic-only. It is not allowed to determine canonical admission/routing/signer semantics.
- Preserve typed failure behavior so current carrier-v1 paths remain product-useful without relying on legacy string classifiers.
- Project missing typed failure facts as `REMOTE_FAILURE_UNTYPED` with `Unavailable` rather than guessing permission, route, or signer classes from substrings.
- Redact custody implementation details from untyped raw failures to avoid exposing keyring internals when a remote peer fails to send structured failure facts.

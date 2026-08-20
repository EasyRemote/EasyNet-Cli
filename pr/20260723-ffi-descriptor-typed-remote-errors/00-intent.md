# Intent

## Goal

Remove message-string classification from the FFI descriptor resolver's remote
catalog probe path.

## Non-goals

- Do not change the public C ABI entry point.
- Do not weaken descriptor-not-found or owner-offline behavior.
- Do not add fallback descriptor lookup paths.

## Acceptance criteria

- The FFI descriptor resolver no longer has a helper that classifies
  `anyhow::Error` by substrings such as `owner is not online`,
  `ROUTE_NEGATIVE`, or `requires a caller signer`.
- Remote descriptor probe construction and invocation return
  `DescriptorResolutionError` directly.
- ABI projection remains owned by `DescriptorResolutionError`.
- Gates reject reintroduction of message-string classification.

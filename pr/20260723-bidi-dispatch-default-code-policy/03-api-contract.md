# API Contract

- Rename `fallback_code` to `default_code`.
- Keep `failed_dispatch_result(reason, code, retryable)` call shape unchanged.
- Do not change public protobuf, SDK, FFI, or CLI interfaces.
- Reject reintroduced fallback vocabulary only in the production helper
  boundary, not in negative tests that prove legacy rejection.

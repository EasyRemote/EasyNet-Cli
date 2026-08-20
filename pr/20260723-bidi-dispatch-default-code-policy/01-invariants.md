# Invariants

- Bidi dispatch failures still produce exactly one terminal `DispatchResult`.
- Failure code extraction remains delegated to `SessionFailure` and
  `FailureCodeClassifier`; this slice does not fork classification logic.
- Caller-selected default codes remain explicit at each failure site.
- No legacy JSON/session carrier path is reintroduced.
- Public wire behavior is unchanged: code, retryability, and reason values are
  preserved.

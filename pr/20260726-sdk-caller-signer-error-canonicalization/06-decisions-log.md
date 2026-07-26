Decisions log
=============

2026-07-26
----------

- Treat raw caller-signer/keyring details as a canonical SDK error projection
  defect, not a product UI formatting problem.
- Keep the rule at the shared SDK error decoder boundary so all providers and
  future transports converge on the same public runtime model.
- Preserve structured runtime error facts while canonicalizing only the public
  message for `CALLER_SIGNER_UNAVAILABLE`; this avoids adding a fallback
  classifier or broad string-rewrite layer.

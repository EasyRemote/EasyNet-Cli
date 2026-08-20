Invariants
==========

1. Caller signer unavailability is a custody/readiness failure, not a keyring
   storage disclosure.
2. SDK error decoding is the canonical language boundary; product callers must
   not need to sanitize transport messages.
3. Error code, stage, retry, source, receipt URA, invocation ID, and details are
   structured facts and must not be rewritten by message canonicalization.
4. The rule is generic runtime terminology only: caller signer, local key
   service, caller URA.
5. No fallback code classification is introduced. Unknown error codes remain
   invalid unless already accepted as canonical extension codes.

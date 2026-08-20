# Node Publication Seam Invariants

1. Publication remains a daemon SDK profile, not a product SDK.
2. Node may validate canonical request field presence, but must not derive
   ResourceRef URAs, descriptor refs, or publication result semantics locally.
3. Deploy, unpublish, enable, and disable carrier builders must return complete
   `InvocationDraft` objects supplied by the injected transport.
4. Requests use only latest canonical snake_case field names.
5. No legacy input aliases are accepted.
6. No method may call a CLI subprocess or direct daemon socket in the seam.
7. Close state is explicit and idempotent.
8. Conformance evidence may mark Node only for cases covered by public seam
   methods and tests.

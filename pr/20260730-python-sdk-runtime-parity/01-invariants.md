# Invariants

1. Public runtime receipts must use generic session authority fields only:
   - `issuer_ura`
   - `subject_ura`
2. Retired public fields `backend_ura` and `user_ura` remain rejected at public receipt boundaries.
3. Internal Axon SDK dataclasses use the current canonical constructor names; EasyNet SDK must not preserve old constructor compatibility.
4. Descriptor-ref action is an admission action. Bidi session transport uses the `stream` admission action; it must not invent a `bidi` descriptor-ref action.
5. Python, Go, and Node SDKs must converge on the same authority/descriptor semantics.


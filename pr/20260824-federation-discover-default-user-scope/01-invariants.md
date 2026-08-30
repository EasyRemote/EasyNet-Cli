# Invariants

1. A Device is placement/custody, not an operator principal.
2. Unfiltered federation directory access is Authority-only and explicitly
   requested.
3. Product directory reads are User-scoped and bind caller, subject, request
   filter, and signer to the same user identity.
4. The CLI selects scope before entering the invocation transport; transport
   code does not infer product intent.
5. Missing or malformed credentials fail closed; they never widen access to an
   operator/audit read.

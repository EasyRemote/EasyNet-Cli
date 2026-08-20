# Invariants

1. A retained `PreparedInvocation` without `prepared_id` fails closed.
2. A material-only response has no native prepared handle.
3. Material-only decoding returns only canonical `SigningMaterial`.
4. Go and Python expose the same two-state prepare model.
5. Browser prepare does not receive a process-local native capability.

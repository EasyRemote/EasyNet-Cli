# Invariants

1. Signer handles never contain private key material.
2. Signer handles are daemon/keyring policy projections; SDK facades validate
   policy facts but do not decide keyring authorization.
3. A PreparedInvocation is never submit-ready.
4. Only a SignedInvocation with a handle-matching signer id crosses submit.
5. The same validation semantics must hold in Go and Python.
6. Public behavior remains product-neutral: no EasyRemote, backend, browser, or
   product session signer concepts are introduced.

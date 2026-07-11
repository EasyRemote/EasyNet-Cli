# Invariants

1. Runtime environment projection is product-neutral SDK state.
2. Products may choose product DTO names, but they must not parse or validate
   runtime credentials independently when the SDK projection exists.
3. The SDK projection must not contain private-key material, keyring paths or
   signing handles.
4. Go and Python expose the same conceptual capability.
5. Missing or malformed credentials are deterministic SDK errors, not silent
   empty identities.

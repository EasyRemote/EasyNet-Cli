# Invariants

1. Runtime caller signer resolution has one canonical entry point.
2. Boot and product helper code must not choose key-service custody classes directly.
3. Device/hub runtime owner behavior remains public-interface compatible.
4. User managed custody remains behind the same resolver, not reimplemented by callers.
5. Trust auto-wire may project a public key, but it must receive it through an owner-bound signer capability.


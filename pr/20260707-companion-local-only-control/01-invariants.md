# Companion Local-Only Invariants

1. Companion control abilities remain callable through local daemon control
   paths.
2. Public catalogue publication uses one shared predicate.
3. Device-profile descriptors must not contain companion control abilities.
4. Route resolution must not treat a LocalRuntime handler as sufficient proof
   that a daemon-local control ability is remotely routable.
5. Existing public device abilities continue to resolve from LocalRuntime
   authority.
6. No product-specific SDK behavior is introduced.

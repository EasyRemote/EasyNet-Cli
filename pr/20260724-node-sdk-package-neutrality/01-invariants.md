# Invariants

1. The SDK is not an EasyNet SDK or an EasyRemote SDK.
2. Private package metadata is still architecture evidence because templates,
   plugin scaffolds, and downstream package references copy it.
3. Public JS/TS exported symbols remain unchanged.
4. Provider paths may remain under explicit provider namespaces; the canonical
   root package identity must stay product-neutral.
5. Metadata gates must enforce the target architecture, not the retired product
   name.

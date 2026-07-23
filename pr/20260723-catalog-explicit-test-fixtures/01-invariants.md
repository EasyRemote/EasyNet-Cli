# Invariants

1. Test-only helpers must require an explicit Device authority URA.
2. No helper may call local daemon identity, key service, host pairing, or
   filesystem credential discovery.
3. Production constructors remain unchanged and fail closed under their current
   authority model.
4. Ability modules should not duplicate catalog authority construction.
5. Public runtime, SDK, CLI, FFI, and daemon APIs remain unchanged.

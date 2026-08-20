# Invariants

1. Runtime host lifecycle is a generic runtime provider capability, not an EasyNet product lifecycle.
2. The SDK must not expose a product credentials directory adapter as a canonical runtime abstraction.
3. No compatibility imports remain under `providers.easynet`.
4. C ABI payloads remain stable while SDK package ownership and naming converge.

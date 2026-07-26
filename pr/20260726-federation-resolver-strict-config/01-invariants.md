# Invariants

1. Resolver configuration is canonical state, not a migration carrier.
2. Unknown fields must not be ignored because that can hide stale operator
   intent and produce partial federation routes.
3. Empty current fields remain valid only where the current architecture allows
   operator-provided endpoint absence.
4. The resolver owns realm classification only; downstream routing owns
   invocation admission and descriptor lookup.

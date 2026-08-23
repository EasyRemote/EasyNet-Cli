# Invariants

1. The compiled builtin plugin ability spec is the registration source for
   call mode and bidi frame representation.
2. The registry manifest must be a lossless projection of that spec.
3. The frontend never infers a binary wire format from an ability name or
   `call_mode = bidi` alone.
4. Runtime execution and catalog advertisement use the same frame contract.
5. No compatibility fallback treats a missing wire kind as executable.

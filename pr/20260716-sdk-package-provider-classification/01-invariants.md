# Invariants

1. Product-branded package roots cannot be certified as `public_facade` or
   `provider_neutral_core`.
2. Product-branded package roots that remain for public compatibility are
   explicitly classified as EasyNet provider/compat surfaces.
3. Go and Python neutral core roots remain the only roots scanned by the
   product-neutrality source gate.
4. The public API inventory and language package names are not renamed in this
   slice.
5. No compatibility fallback is added to runtime code.

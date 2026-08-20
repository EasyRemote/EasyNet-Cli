# Intent

Close the SDK public-surface source fork where `legacy_quarantine` metadata
could describe arbitrary non-canonical symbols without proving that the symbol
is actually rejected by the canonical product-neutral policy.

The canonical SDK model must have one reason source for product/provider
exports: `sdk_public_surface_policy.canonical_quarantine_reason`.

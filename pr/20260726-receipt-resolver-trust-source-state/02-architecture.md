# Architecture

`CanonicalRuntimeReceiptResolver` now owns a small trust-source state machine:

1. `Loaded`: a non-empty `RealmTrustAnchorKeyResolver` is available.
2. `Empty`: the configured trust anchor loaded but contains no trusted rows.
3. `LoadFailed`: reading or parsing the configured trust anchor failed.

Resolution remains local-first, realm-trust-second. The difference is that the second stage now preserves the reason it cannot participate instead of collapsing all cases into an availability fallback.

# Decisions Log

## 2026-07-07

- Kept Python descriptor-ref decomposition behind transport-backed projection for callers that need canonical daemon/Axon facts.
- Kept descriptor-ref canonical construction and projection in identity/profile helpers; Runtime Core builder validates only tuple field presence and payload carriers.
- Used an injectable Python Addressing facade because descriptor projection should be reusable in tests without making draft construction parse Axon grammar locally.

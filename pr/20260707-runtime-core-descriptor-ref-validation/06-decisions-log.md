# Decisions Log

## 2026-07-07

- Split Python descriptor-ref behavior into local shape validation for InvocationBuilder and transport-backed projection for callers that need canonical daemon/Axon facts.
- Kept descriptor-ref canonical construction and projection in identity/profile helpers; Runtime Core builder validates the offline carrier shape required for tuple completeness.
- Used an injectable Python Addressing facade because the Python builder cannot require a daemon identity transport without coupling draft construction to process liveness.

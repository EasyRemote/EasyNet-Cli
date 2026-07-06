# Decisions Log

## 2026-07-07

- Kept descriptor-ref canonical construction and projection in identity/profile helpers; Runtime Core builder validates tuple completeness but does not own descriptor-ref grammar.
- Used an injectable Python Addressing facade because the Python builder cannot require a daemon identity transport without coupling draft construction to process liveness.
- Rejected a local descriptor-ref shape validator: callers that need canonical daemon/Axon facts use `parse_ability_descriptor_ref(..., addressing=...)` or the default Identity/Addressing C ABI facade.

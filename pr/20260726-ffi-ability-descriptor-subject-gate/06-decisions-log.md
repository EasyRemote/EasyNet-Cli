# Decisions Log

## 2026-07-26

- Treat `AbilityDescriptor` subject validation as a provider-kind responsibility, matching the existing receipt-history provider gate.
- Require the catalogue read subject to be the callee realm authority URA.
- Do not add product-specific exceptions for EasyNet devices or EasyRemote hubs.
- Do not preserve device-subject catalogue reads as compatibility behavior.
- Extend the SPEC v2 gate so the removed `AbilityDescriptor => Ok(())` behavior cannot return without failing convergence checks.

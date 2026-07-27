# Decisions Log

## 2026-07-27

- Selected a narrow RF-1 product-neutrality leak inside the active Python
  runtime transport. The Python distribution facade remains product-branded in
  this checkout and is already classified separately by conformance; this slice
  does not add another import compatibility layer.
- Replaced active worker thread names from `easynet-sdk-*` to
  `runtime-sdk-*` and centralized the names as transport module constants.
- Added an ownership test so product-branded runtime worker names cannot
  re-enter the provider-neutral transport implementation.
